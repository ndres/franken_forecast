//! Cross-language Python <-> Rust golden-vector parity (`<= 1e-9`, `§0.2` Rule 6),
//! `10,000`-case `proptest` suites across all numerical modules, and JSON Schema contract validation.

use fcast_core::belief::{
    ConjugateBelief, Direction, DirichletCategoricalBelief, DomainKind, kalman_market_blend,
    prior_shift_prob,
};
use fcast_core::blinding::{MarketBlindingScrubber, VerifiedEvidenceRef};
use fcast_core::coherence::{
    build_metaculus_cdf, eb_shrink_base_rate, effective_model_count, pav_isotonic_nondecreasing,
};
use fcast_core::forecast::{
    AttributionWaterfall, HorizonGeometry, OutcomeDistribution, ProbClamped,
    ResolutionStatusAtCutoff, UnsealedForecastRevision,
};
use proptest::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;

fn assert_close(actual: f64, expected: f64, label: &str) {
    let diff = (actual - expected).abs();
    let tol = 1e-9 * expected.abs().max(1.0);
    assert!(
        diff <= tol,
        "{label}: actual={actual} expected={expected} diff={diff:.3e} > tol={tol:.3e}"
    );
}

#[test]
fn golden_python_rust_math_parity_1e9() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let raw = fs::read_to_string(root.join("tests/golden/math_vectors.json"))
        .expect("read golden math_vectors.json");
    let v: Value = serde_json::from_str(&raw).expect("parse golden json");

    let up = ConjugateBelief {
        alpha: 8.0,
        beta: 2.0,
    }
    .stream_step(
        0.50,
        4.0,
        DomainKind::Geopolitics,
        HorizonGeometry::PointInTime,
        0.0,
        500.0,
        1000.0,
        0.5,
        Direction::Up,
        0.25,
        0.6,
        1.0,
        1.35,
        0.6,
    );
    assert_close(
        up.mean(),
        v["stream_up"]["mean"].as_f64().expect("f"),
        "stream_up.mean",
    );
    assert_close(
        up.n_eff(),
        v["stream_up"]["n_eff"].as_f64().expect("f"),
        "stream_up.n_eff",
    );
    assert_close(
        up.variance(),
        v["stream_up"]["var"].as_f64().expect("f"),
        "stream_up.var",
    );

    let dead = ConjugateBelief {
        alpha: 1.2,
        beta: 8.8,
    }
    .stream_step(
        0.12,
        10.0,
        DomainKind::Geopolitics,
        HorizonGeometry::CumulativeStoppingTime,
        500.0,
        36.0,
        8760.0,
        0.5,
        Direction::Still,
        0.0,
        0.0,
        1.0,
        1.35,
        0.6,
    );
    assert_close(
        dead.mean(),
        v["stream_deadline"]["mean"].as_f64().expect("f"),
        "stream_deadline.mean",
    );

    let shk = ConjugateBelief {
        alpha: 1.25,
        beta: 23.75,
    }
    .stream_step(
        0.05,
        5.0,
        DomainKind::Geopolitics,
        HorizonGeometry::PointInTime,
        1.0,
        300.0,
        600.0,
        2.8,
        Direction::Up,
        2.2,
        0.9,
        1.0,
        1.35,
        0.6,
    );
    assert_close(
        shk.mean(),
        v["stream_shock"]["mean"].as_f64().expect("f"),
        "stream_shock.mean",
    );
    assert_close(
        shk.n_eff(),
        v["stream_shock"]["n_eff"].as_f64().expect("f"),
        "stream_shock.n_eff",
    );

    assert_close(
        prior_shift_prob(0.70, 0.50, 0.01),
        v["king_zeng_070_050_001"].as_f64().expect("f"),
        "king_zeng",
    );

    let kd = kalman_market_blend(
        ProbClamped::new(0.40).expect("p"),
        0.25,
        ProbClamped::new(0.50).expect("p"),
        0.90,
        0.10,
    );
    assert_close(
        kd.p_post.get(),
        v["kalman_deep"]["p_post"].as_f64().expect("f"),
        "kalman_deep.p_post",
    );
    assert_close(
        kd.bayesian_surprise_nats,
        v["kalman_deep"]["bayesian_surprise_nats"]
            .as_f64()
            .expect("f"),
        "kalman_deep.kl",
    );

    let kt = kalman_market_blend(
        ProbClamped::new(0.40).expect("p"),
        0.25,
        ProbClamped::new(0.50).expect("p"),
        0.01,
        0.10,
    );
    assert_close(
        kt.bayesian_surprise_nats,
        v["kalman_thin"]["bayesian_surprise_nats"]
            .as_f64()
            .expect("f"),
        "kalman_thin.kl",
    );

    let cdf_lin = build_metaculus_cdf(
        &[(20.0, 0.10), (50.0, 0.50), (80.0, 0.90)],
        0.0,
        100.0,
        false,
        false,
        None,
        None,
        5e-5,
        0.59,
    )
    .expect("lin");
    let exp_lin = v["cdf_linear_201"].as_array().expect("arr");
    assert_eq!(cdf_lin.len(), exp_lin.len());
    for (i, (&a, e)) in cdf_lin.iter().zip(exp_lin.iter()).enumerate() {
        assert_close(a, e.as_f64().expect("f"), &format!("cdf_lin[{i}]"));
    }

    let cdf_log = build_metaculus_cdf(
        &[(10.0, 0.15), (100.0, 0.55), (1000.0, 0.85)],
        1.0,
        10000.0,
        true,
        true,
        Some(0.0),
        None,
        5e-5,
        0.59,
    )
    .expect("log");
    let exp_log = v["cdf_log_201"].as_array().expect("arr");
    for (i, (&a, e)) in cdf_log.iter().zip(exp_log.iter()).enumerate() {
        assert_close(a, e.as_f64().expect("f"), &format!("cdf_log[{i}]"));
    }

    let pav = pav_isotonic_nondecreasing(&[0.30, 0.24, 0.50, 0.48]);
    let exp_pav = v["pav_mono"].as_array().expect("arr");
    for (i, (&a, e)) in pav.iter().zip(exp_pav.iter()).enumerate() {
        assert_close(a, e.as_f64().expect("f"), &format!("pav[{i}]"));
    }

    assert_close(
        eb_shrink_base_rate(0, 3, 0.20, 5.0),
        v["eb_shrink_0_3_020_5"].as_f64().expect("f"),
        "eb",
    );
    assert_close(
        effective_model_count(4, 0.5),
        v["m_eff_4_05"].as_f64().expect("f"),
        "m_eff",
    );
}

#[test]
fn forecast_revision_matches_committed_json_schema_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(root.join("schemas/v1/forecast_revision.schema.json"))
            .expect("read schema"),
    )
    .expect("parse schema");
    let required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|s| s.as_str().expect("str"))
        .collect();

    let rev = UnsealedForecastRevision {
        forecast_id: "fc_schema_test".into(),
        seq_no: 0,
        parent_revision_id: None,
        question_id: "q_schema".into(),
        as_of_utc: "2026-11-25T12:00:00Z".into(),
        p_indep: ProbClamped::new(0.42).expect("p"),
        p_market: Some(ProbClamped::new(0.38).expect("p")),
        probability_distribution: OutcomeDistribution::Binary {
            p: ProbClamped::new(0.40).expect("p"),
            n_eff: 10.0,
        },
        evidence_handles: vec![VerifiedEvidenceRef::parse("[ev:deadbeef#span1]").expect("ev")],
        model_version: "gemini-4-argon@2026-05-20".into(),
        calibration_version: "cal-v1".into(),
        rationale_handle: VerifiedEvidenceRef::parse("[ev:cafebabe#rat0]").expect("rat"),
        resolution_status: ResolutionStatusAtCutoff::Unresolved,
    }
    .seal()
    .expect("seal");
    let obj = serde_json::to_value(&rev).expect("serialize");
    let map = obj.as_object().expect("json object");

    // Exact field parity with additionalProperties: false
    assert_eq!(map.len(), required.len());
    for key in required {
        assert!(map.contains_key(key), "Missing schema-required key {key}");
    }

    // All 4 market-blind prompt templates must pass MarketBlindingScrubber
    for tmpl in [
        "rationale_staged_turn_a.tmpl",
        "stream_likelihood_tick.tmpl",
        "belief_fold_step.tmpl",
        "red_team_audit.tmpl",
    ] {
        let body = fs::read_to_string(root.join("templates").join(tmpl)).expect("read tmpl");
        assert!(
            MarketBlindingScrubber::assert_blind_payload(&body).is_ok(),
            "Market-blind template {tmpl} failed MarketBlindingScrubber!"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn prop_pav_is_monotone_and_idempotent(vals in prop::collection::vec(0.0_f64..1.0, 2..12)) {
        let m1 = pav_isotonic_nondecreasing(&vals);
        let m2 = pav_isotonic_nondecreasing(&m1);
        for win in m1.windows(2) {
            prop_assert!(win[0] <= win[1] + 1e-12);
        }
        for (a, b) in m1.iter().zip(m2.iter()) {
            prop_assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn prop_dirichlet_simplex_and_still_invariance(
        w0 in 0.05_f64..1.0,
        w1 in 0.05_f64..1.0,
        w2 in 0.05_f64..1.0,
        d0 in -2.0_f64..2.0,
        d1 in -2.0_f64..2.0,
        d2 in -2.0_f64..2.0,
    ) {
        let s = w0 + w1 + w2;
        let b = DirichletCategoricalBelief {
            probs: vec![w0 / s, w1 / s, w2 / s],
            n_eff: 8.0,
        };
        let still = b.step(&[d0, d1, d2], 1.0, 1.35, true);
        prop_assert_eq!(still, b.clone());
        let moved = b.step(&[d0, d1, d2], 0.8, 1.35, false);
        let sum: f64 = moved.probs.iter().sum();
        prop_assert!((sum - 1.0).abs() < 1e-11);
    }

    #[test]
    fn prop_kalman_surprise_nonnegative_and_monotone_in_liquidity(
        p_ind in 0.05_f64..0.95,
        p_mkt in 0.05_f64..0.95,
        phi_lo in 0.01_f64..0.45,
        phi_hi in 0.50_f64..1.0,
    ) {
        let a = kalman_market_blend(ProbClamped::clamp_lossy(p_ind), 0.25, ProbClamped::clamp_lossy(p_mkt), phi_lo, 0.10);
        let b = kalman_market_blend(ProbClamped::clamp_lossy(p_ind), 0.25, ProbClamped::clamp_lossy(p_mkt), phi_hi, 0.10);
        prop_assert!(a.bayesian_surprise_nats >= 0.0);
        prop_assert!(b.bayesian_surprise_nats + 1e-12 >= a.bayesian_surprise_nats);
    }

    #[test]
    fn prop_waterfall_exact_on_interior(
        p0 in 0.05_f64..0.95,
        d1 in -0.5_f64..0.5,
        d2 in -0.5_f64..0.5,
        d3 in -0.5_f64..0.5,
        d4 in -0.5_f64..0.5,
        d5 in -0.5_f64..0.5,
    ) {
        let wf = AttributionWaterfall { decay: d1, stream: d2, fold: d3, red_team: d4, kalman: d5 };
        prop_assert!(wf.apply_and_verify(ProbClamped::clamp_lossy(p0)).is_ok());
    }

    #[test]
    fn prop_metaculus_cdf_201_always_valid(
        p10 in 0.02_f64..0.25,
        p50 in 0.30_f64..0.70,
        p90 in 0.75_f64..0.98,
        open_lo in any::<bool>(),
        open_hi in any::<bool>(),
    ) {
        let cdf = build_metaculus_cdf(
            &[(20.0, p10), (50.0, p50), (80.0, p90)],
            0.0, 100.0, open_lo, open_hi, None, None, 5e-5, 0.59,
        ).expect("cdf");
        prop_assert_eq!(cdf.len(), 201);
        for win in cdf.windows(2) {
            prop_assert!(win[1] - win[0] >= 5e-5 - 1e-9);
            prop_assert!(win[1] - win[0] <= 0.59 + 1e-9);
        }
    }
}
