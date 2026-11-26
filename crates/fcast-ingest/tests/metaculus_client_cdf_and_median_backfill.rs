//! `GATE-11` integration tests: Metaculus question hierarchy, 201-pt/discrete/log/date CDFs,
//! fixed-point MC simplex + `restore_option_names`, `ApiFilter`, `ManticCrucibleAdapter`,
//! and `ForecastBenchMedianBackfill` 3-tier crowd prior cascade.

use std::collections::BTreeMap;

use fcast_ingest::{
    ApiFilter, BinaryQuestion, BotLeaderboardStatus, ConditionalQuestion, CrowdPriorSource,
    DEFAULT_MC_SIMPLEX_FLOOR, DateQuestion, DiscreteQuestion, ForecastBenchMedianBackfill,
    ForecastBenchMedianRow, GroupOfQuestions, MANTIC_OPEN_TAIL_FLOOR, METACULUS_CDF_MAX_STEP,
    METACULUS_CDF_MIN_STEP, ManticCrucibleAdapter, MetaculusError, MetaculusQuestion,
    MultipleChoiceQuestion, NumericQuestion, QuestionMeta, enforce_mantic_open_tail_floor,
    fixed_point_simplex_floor, restore_option_names,
};
use serde_json::json;

#[test]
fn gate11_metaculus_cdf_linear_log_discrete_and_date() {
    // 1. Continuous 201-pt linear closed bounds
    let lin_q = NumericQuestion {
        meta: QuestionMeta {
            id: 1001,
            title: "US CPI YoY Q4 2026".into(),
            ..QuestionMeta::default()
        },
        lower_bound: 0.0,
        upper_bound: 100.0,
        open_lower_bound: false,
        open_upper_bound: false,
        zero_point: None,
        unit: Some("%".into()),
    };
    let lin = lin_q
        .build_cdf(&[(20.0, 0.10), (50.0, 0.50), (80.0, 0.90)])
        .expect("linear cdf");
    assert_eq!(lin.len(), 201);
    assert_eq!(lin[0], 0.0);
    assert_eq!(lin[200], 1.0);
    assert!(lin.windows(2).all(|w| {
        let d = w[1] - w[0];
        (METACULUS_CDF_MIN_STEP - 1e-12..=METACULUS_CDF_MAX_STEP + 1e-12).contains(&d)
    }));

    // 2. Continuous 201-pt log-scaled (zero_point = Some(0.0)) open bounds
    let log_q = NumericQuestion {
        meta: QuestionMeta {
            id: 1002,
            title: "Annual ransomware ransom total (USD M)".into(),
            ..QuestionMeta::default()
        },
        lower_bound: 10.0,
        upper_bound: 10_000.0,
        open_lower_bound: true,
        open_upper_bound: true,
        zero_point: Some(0.0),
        unit: Some("USD M".into()),
    };
    let log_cdf = log_q
        .build_cdf(&[(50.0, 0.10), (500.0, 0.50), (4_000.0, 0.90)])
        .expect("log cdf");
    assert_eq!(log_cdf.len(), 201);
    assert!(log_cdf[0] >= 0.001 - 1e-12);
    assert!(log_cdf[200] <= 0.999 + 1e-12);
    assert!(log_cdf.windows(2).all(|w| {
        let d = w[1] - w[0];
        (METACULUS_CDF_MIN_STEP - 1e-12..=METACULUS_CDF_MAX_STEP + 1e-12).contains(&d)
    }));

    // 3. Discrete question: inbound_outcome_count = 15 -> 16-point CDF
    let disc_q = DiscreteQuestion {
        meta: QuestionMeta {
            id: 1003,
            title: "FOMC 25bp cuts in 2026".into(),
            ..QuestionMeta::default()
        },
        lower_bound: 0.0,
        upper_bound: 15.0,
        open_lower_bound: false,
        open_upper_bound: true,
        inbound_outcome_count: 15,
    };
    let disc = disc_q
        .build_cdf(&[(3.0, 0.20), (8.0, 0.60), (12.0, 0.90)])
        .expect("discrete cdf");
    assert_eq!(disc.len(), 16);
    assert_eq!(disc[0], 0.0);
    assert!(disc[15] <= 0.999 + 1e-12);

    // 4. Open-bounded DateQuestion with far-tail date compression
    let date_q = DateQuestion {
        meta: QuestionMeta {
            id: 1004,
            title: "Date of next ceasefire agreement".into(),
            ..QuestionMeta::default()
        },
        lower_bound_epoch_s: 1_767_225_600.0, // 2026-01-01
        upper_bound_epoch_s: 1_798_761_600.0, // 2027-01-01
        open_lower_bound: true,
        open_upper_bound: true,
    };
    let date_cdf = date_q
        .build_cdf(&[
            (1_760_000_000.0, 0.02),
            (1_780_000_000.0, 0.50),
            (1_850_000_000.0, 0.98),
        ])
        .expect("date cdf");
    assert_eq!(date_cdf.len(), 201);
    assert!(date_cdf[0] >= 0.001 - 1e-12);
    assert!(date_cdf[200] <= 0.999 + 1e-12);
}

#[test]
fn gate11_multiple_choice_fixed_point_simplex_and_restore_option_names() {
    let floored =
        fixed_point_simplex_floor(&[0.995, 0.002, 0.002, 0.001], DEFAULT_MC_SIMPLEX_FLOOR)
            .expect("simplex floor");
    let sum: f64 = floored.iter().sum();
    assert!((sum - 1.0).abs() < 1e-12);
    assert!(floored.iter().all(|&p| p >= 0.01 - 1e-12));

    let canonical = vec![
        "10 \u{2013} 20%".to_string(),
        "20 \u{2013} 30%".to_string(),
        "Above \u{201C}30%\u{201D}".to_string(),
    ];
    let mut mangled = BTreeMap::new();
    mangled.insert("10 - 20%".to_string(), 0.994);
    mangled.insert("  20 — 30% ".to_string(), 0.004);
    mangled.insert("above \"30%\"".to_string(), 0.002);

    let restored = restore_option_names(&mangled, &canonical).expect("restore names");
    assert!(restored.contains_key("10 \u{2013} 20%"));
    assert!(restored.contains_key("20 \u{2013} 30%"));
    assert!(restored.contains_key("Above \u{201C}30%\u{201D}"));

    let mc = MultipleChoiceQuestion {
        meta: QuestionMeta {
            id: 2001,
            title: "Tariff band".into(),
            ..QuestionMeta::default()
        },
        options: canonical.clone(),
    };
    let final_dist = mc
        .normalize_and_restore(&mangled, DEFAULT_MC_SIMPLEX_FLOOR)
        .expect("normalize and restore");
    assert_eq!(final_dist.len(), 3);
    assert!((final_dist.values().sum::<f64>() - 1.0).abs() < 1e-12);
    assert!(final_dist.values().all(|&p| p >= 0.01 - 1e-12));

    // Reject unknown option label
    let mut bad = BTreeMap::new();
    bad.insert("totally_unknown".to_string(), 1.0);
    assert!(matches!(
        restore_option_names(&bad, &canonical),
        Err(MetaculusError::UnknownOptionLabel(_))
    ));
}

#[test]
fn gate11_conditional_group_and_api_filter_auto_discovery() {
    let b_yes = MetaculusQuestion::Binary(BinaryQuestion {
        meta: QuestionMeta {
            id: 3001,
            title: "Child Yes".into(),
            ..QuestionMeta::default()
        },
    });
    let b_no = MetaculusQuestion::Binary(BinaryQuestion {
        meta: QuestionMeta {
            id: 3002,
            title: "Child No".into(),
            ..QuestionMeta::default()
        },
    });
    let cond = MetaculusQuestion::Conditional(ConditionalQuestion {
        meta: QuestionMeta {
            id: 3000,
            title: "Conditional rate cut".into(),
            ..QuestionMeta::default()
        },
        condition_id: 2999,
        condition_child_id: 3000,
        question_yes: Box::new(b_yes.clone()),
        question_no: Box::new(b_no.clone()),
    });
    let group = MetaculusQuestion::Group(GroupOfQuestions {
        meta: QuestionMeta {
            id: 3010,
            title: "Group of subquestions".into(),
            ..QuestionMeta::default()
        },
        subquestions: vec![b_yes, b_no],
        exhaustive: true,
    });
    assert_eq!(cond.meta().id, 3000);
    assert_eq!(group.meta().id, 3010);

    let filter = ApiFilter::auto_discover_prize_tournaments();
    assert!(filter.matches_tournament(BotLeaderboardStatus::BotsOnly, true));
    assert!(filter.matches_tournament(BotLeaderboardStatus::Include, true));
    assert!(!filter.matches_tournament(BotLeaderboardStatus::Exclude, true));
    assert!(!filter.matches_tournament(BotLeaderboardStatus::BotsOnly, false));
    let params = filter.to_query_params();
    assert!(
        params
            .iter()
            .any(|(k, v)| k == "bot_leaderboard_status" && v == "bots_only")
    );
    assert!(
        params
            .iter()
            .any(|(k, v)| k == "bot_leaderboard_status" && v == "include")
    );
}

#[test]
fn gate11_mantic_crucible_adapter_rewrite_and_open_tail_floor() {
    let adapter = ManticCrucibleAdapter::default();

    // Rewrite Mantic `quantitative` wire payload onto NumericQuestion
    let quant_payload = json!({
        "id": 9001,
        "type": "quantitative",
        "title": "Eurozone inflation 2026",
        "lower_bound": 0.0,
        "upper_bound": 10.0,
        "open_lower_bound": true,
        "open_upper_bound": true,
        "unit": "%"
    });
    let rewritten = adapter
        .rewrite_wire_question(&quant_payload)
        .expect("rewrite quantitative");
    assert!(matches!(rewritten, MetaculusQuestion::Numeric(_)));

    // Empty allowed_types preserved and offset clamped to ceiling
    let raw_filter = ApiFilter {
        allowed_types: Vec::new(),
        offset: Some(50_000),
        ..ApiFilter::default()
    };
    let norm = adapter.normalize_filter(&raw_filter);
    assert!(norm.allowed_types.is_empty());
    assert_eq!(norm.offset, Some(adapter.max_offset));

    // Pointwise mean across ensemble legs + >= 5% open-tail floor
    let legs = vec![
        vec![0.004, 0.490, 0.502, 0.004],
        vec![0.006, 0.490, 0.498, 0.006],
    ];
    let floored = adapter
        .aggregate_and_floor_pmfs(&legs, true, true)
        .expect("aggregate and floor");
    assert_eq!(floored.len(), 4);
    assert!((floored.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    assert!(floored[0] >= MANTIC_OPEN_TAIL_FLOOR - 1e-12);
    assert!(floored[3] >= MANTIC_OPEN_TAIL_FLOOR - 1e-12);

    // Reject N > 31 bins
    let too_many = vec![1.0 / 32.0; 32];
    assert!(matches!(
        enforce_mantic_open_tail_floor(&too_many, true, true, MANTIC_OPEN_TAIL_FLOOR),
        Err(MetaculusError::InvalidManticPmf(_))
    ));
}

#[test]
fn gate11_forecastbench_median_backfill_three_tier_cascade_and_blinding() {
    let fb_valid = ForecastBenchMedianRow {
        question_id: "q_meta_1".into(),
        available_utc: "2026-11-15T00:00:00Z".into(),
        freeze_crowd_median: Some(0.34),
        sf_median: Some(0.31),
        public_median: Some(0.33),
    };
    let fb_future = ForecastBenchMedianRow {
        question_id: "q_meta_1".into(),
        available_utc: "2026-11-25T00:00:00Z".into(),
        freeze_crowd_median: None,
        sf_median: Some(0.29),
        public_median: None,
    };

    // Tier 1: Live liquid unmasked venue
    let t1 = ForecastBenchMedianBackfill::resolve(
        Some(0.38),
        false,
        Some(&fb_valid),
        Some(0.36),
        "2026-11-18T00:00:00Z",
        false,
    )
    .expect("tier 1");
    assert_eq!(t1.source, CrowdPriorSource::VenueLive);
    assert!((t1.prob.get() - 0.38).abs() < 1e-9);

    // Tier 2: Gateway-masked Metaculus recovers ForecastBench published median when available_utc <= as_of_utc
    let t2 = ForecastBenchMedianBackfill::resolve(
        None,
        true,
        Some(&fb_valid),
        Some(0.36),
        "2026-11-18T00:00:00Z",
        false,
    )
    .expect("tier 2");
    assert_eq!(t2.source, CrowdPriorSource::ForecastBenchMedian);
    assert!((t2.prob.get() - 0.34).abs() < 1e-9);

    // Tier 3: Post-cutoff ForecastBench row blocked by INV-03 fence -> falls back to cluster consensus
    let t3 = ForecastBenchMedianBackfill::resolve(
        None,
        true,
        Some(&fb_future),
        Some(0.36),
        "2026-11-18T00:00:00Z",
        false,
    )
    .expect("tier 3");
    assert_eq!(t3.source, CrowdPriorSource::ClusterConsensus);
    assert!((t3.prob.get() - 0.36).abs() < 1e-9);

    // INV-04 pre-seal blinding quarantine: always None
    let blinded = ForecastBenchMedianBackfill::resolve(
        Some(0.38),
        false,
        Some(&fb_valid),
        Some(0.36),
        "2026-11-18T00:00:00Z",
        true,
    );
    assert!(blinded.is_none());
}
