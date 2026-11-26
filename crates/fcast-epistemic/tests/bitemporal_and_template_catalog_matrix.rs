//! `GATE-08` integration test: TemplateCatalog SHA-256 + blinding matrix, BiTemporalAudit
//! (causal fence, union-strictness, `Q = empty` nullability, `[HH:MM:00Z, HH:MM:59.999Z]` joins),
//! and `ValueExtractionLadder` + `THIN_PUBLISH_FLOOR` (`§9`, `§11.2`).

use std::fs;
use std::path::PathBuf;

use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::forecast::ResolutionStatusAtCutoff;
use fcast_epistemic::{
    AuditEvidenceItem, BiTemporalAudit, ExtractionRung, LOCKED_TEMPLATE_SPECS,
    QuestionContractSpec, TemplateCatalog, TemplateId, ValueExtractionLadder, VerifyError,
    apply_publish_floor, parse_utc_millis,
};
use tempfile::TempDir;

fn workspace_templates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("templates")
}

#[test]
fn template_catalog_loads_all_9_locked_digests_and_enforces_blinding() {
    let catalog = TemplateCatalog::load_and_verify(&workspace_templates_dir())
        .expect("all 9 canonical templates must match §9.2 SHA-256 digests and blinding rules");
    assert_eq!(catalog.len(), 9);

    let bitemp = catalog
        .get(TemplateId::BitemporalEventAudit)
        .expect("bitemporal template present");
    assert_eq!(
        bitemp.sha256_hex,
        "362d5386c85123d3fd491ee7d20da87c6055fa944a7b138253fc7d214b5d1f52"
    );
    assert_eq!(bitemp.short_tag(), "tmpl:362d5386");

    let red_team = catalog
        .get(TemplateId::RedTeamAudit)
        .expect("red_team_audit.tmpl present");
    assert_eq!(
        red_team.sha256_hex,
        "061613585fb855e40a905933ab57bb8ce4ace7a33f670e4e562c954cb527b423"
    );

    // Market-blind templates must never carry `{p_market}` placeholder; Turn A must never carry
    // `{forecaster_probability_distribution}`.
    for id in [
        TemplateId::RationaleStagedTurnA,
        TemplateId::StreamLikelihoodTick,
        TemplateId::BeliefFoldStep,
        TemplateId::RedTeamAudit,
    ] {
        let tmpl = catalog.get(id).expect("blind template exists");
        assert!(
            !tmpl.body.contains("{p_market}"),
            "blind template {} must not contain {{p_market}}",
            tmpl.filename
        );
    }
    let turn_a = catalog
        .get(TemplateId::RationaleStagedTurnA)
        .expect("turn A template exists");
    assert!(
        !turn_a
            .body
            .contains("{forecaster_probability_distribution}")
    );
}

#[test]
fn template_catalog_rejects_digest_tampering_and_blinding_violations() {
    let tmp = TempDir::new().expect("tempdir");
    let src_dir = workspace_templates_dir();
    for spec in LOCKED_TEMPLATE_SPECS {
        fs::copy(src_dir.join(spec.filename), tmp.path().join(spec.filename))
            .expect("copy template");
    }

    // Tamper with red_team_audit.tmpl -> must fail SHA-256 check.
    let tampered_path = tmp.path().join("red_team_audit.tmpl");
    let orig = fs::read_to_string(&tampered_path).expect("read");
    fs::write(&tampered_path, format!("{orig}\n# tampered")).expect("write");
    let err = TemplateCatalog::load_and_verify(tmp.path())
        .expect_err("tampered template must fail digest verification");
    assert!(matches!(
        err,
        VerifyError::TemplateDigestMismatch {
            file: "red_team_audit.tmpl",
            ..
        }
    ));

    // Mechanical variable-level blinding check rejects injected `{p_market}` on blind templates.
    let turn_a_spec = LOCKED_TEMPLATE_SPECS
        .iter()
        .find(|s| s.id == TemplateId::RationaleStagedTurnA)
        .expect("turn A spec");
    let leak_err = TemplateCatalog::verify_blinding(
        turn_a_spec,
        "Question {question_id} {as_of_utc} {contract_sha256} {rationale_handle} {rationale_text} {p_market}",
    )
    .expect_err("injected {p_market} must fail blinding check");
    assert_eq!(
        leak_err,
        VerifyError::TemplateBlindingViolation {
            file: "rationale_staged_turn_a.tmpl",
            placeholder: "{p_market}",
        }
    );
}

#[test]
fn bitemporal_audit_enforces_causal_fence_union_strictness_and_empty_q() {
    let contracts = vec![
        QuestionContractSpec {
            question_id: "q_launched".to_owned(),
            venue: "polymarket".to_owned(),
            contract_sha256: "sha_q1".to_owned(),
            rules_summary: "Missiles launched".to_owned(),
        },
        QuestionContractSpec {
            question_id: "q_entered_airspace".to_owned(),
            venue: "metaculus".to_owned(),
            contract_sha256: "sha_q2".to_owned(),
            rules_summary: "Missiles entered airspace".to_owned(),
        },
    ];

    let items = vec![
        AuditEvidenceItem {
            ev_handle: Some(VerifiedEvidenceRef::parse("[ev:01010101#sig]").expect("handle")),
            published_utc: "2026-11-10 22:30".to_owned(),
            event_utc: None,
            available_utc: "2026-11-10T22:31:00Z".to_owned(),
            who_or_outlet: "OSINT Radar".to_owned(),
            source_type: "other".to_owned(),
            satisfies_rules: false,
            qualified_question_ids: vec![],
            quote: "Preparations observed".to_owned(),
        },
        AuditEvidenceItem {
            ev_handle: Some(VerifiedEvidenceRef::parse("[ev:02020202#wire]").expect("handle")),
            published_utc: "2026-11-10 22:45".to_owned(),
            event_utc: Some("2026-11-10 22:40".to_owned()),
            available_utc: "2026-11-10T22:46:12Z".to_owned(),
            who_or_outlet: "Reuters".to_owned(),
            source_type: "wire flash".to_owned(),
            satisfies_rules: true,
            qualified_question_ids: vec!["q_launched".to_owned()],
            quote: "Missiles launched at 22:40 UTC".to_owned(),
        },
        AuditEvidenceItem {
            ev_handle: Some(VerifiedEvidenceRef::parse("[ev:03030303#mod]").expect("handle")),
            published_utc: "2026-11-10 23:05".to_owned(),
            event_utc: Some("2026-11-10 23:02".to_owned()),
            available_utc: "2026-11-10T23:06:00Z".to_owned(),
            who_or_outlet: "Ministry of Defense".to_owned(),
            source_type: "official statement".to_owned(),
            satisfies_rules: true,
            qualified_question_ids: vec!["q_launched".to_owned(), "q_entered_airspace".to_owned()],
            quote: "Interceptions confirmed inside airspace".to_owned(),
        },
    ];

    // Union-strictness: first_public must pick the 22:45 item (least strict contract q_launched),
    // while per_question_resolution resolves q_launched at 22:45 and q_entered_airspace at 23:05.
    let outcome = BiTemporalAudit::audit("2026-11-10T23:30:00Z", &contracts, &items)
        .expect("valid bi-temporal audit");
    let fp = outcome.first_public.expect("first_public present");
    assert_eq!(fp.published_utc, "2026-11-10 22:45");
    assert_eq!(fp.who_or_outlet, "Reuters");
    assert_eq!(outcome.earlier_signals.len(), 1);
    assert_eq!(outcome.earlier_signals[0].who_or_outlet, "OSINT Radar");
    assert_eq!(outcome.per_question_resolution.len(), 2);
    assert_eq!(
        outcome.per_question_resolution[0].resolution_status,
        ResolutionStatusAtCutoff::Resolved
    );
    assert_eq!(
        outcome.per_question_resolution[0]
            .resolved_published_utc
            .as_deref(),
        Some("2026-11-10 22:45")
    );
    assert_eq!(
        outcome.per_question_resolution[1]
            .resolved_published_utc
            .as_deref(),
        Some("2026-11-10 23:05")
    );

    // Causal fence: cutting off at 23:00:00Z must reject the item with available_utc = 23:06:00Z.
    let fence_err = BiTemporalAudit::audit("2026-11-10T23:00:00Z", &contracts, &items)
        .expect_err("available_utc > as_of_utc must fail");
    assert!(matches!(
        fence_err,
        VerifyError::TemporalFenceViolation { .. }
    ));

    // Q = empty nullability: empty question_contracts yields first_public = None.
    let empty_q = BiTemporalAudit::audit("2026-11-10T23:30:00Z", &[], &items)
        .expect("empty Q must succeed with null first_public");
    assert!(empty_q.first_public.is_none());
    assert!(empty_q.per_question_resolution.is_empty());
}

#[test]
fn bitemporal_minute_window_lead_lag_joins_are_exact() {
    let win = BiTemporalAudit::minute_join_window("2026-11-10 22:45").expect("window");
    let before = parse_utc_millis("2026-11-10T22:44:59.999Z").expect("ts");
    let start = parse_utc_millis("2026-11-10T22:45:00.000Z").expect("ts");
    let mid = parse_utc_millis("2026-11-10T22:45:37.500Z").expect("ts");
    let end = parse_utc_millis("2026-11-10T22:45:59.999Z").expect("ts");
    let after = parse_utc_millis("2026-11-10T22:46:00.000Z").expect("ts");

    assert!(!win.contains(before));
    assert_eq!(win.signed_offset_ms(before), -1);
    assert!(win.contains(start));
    assert_eq!(win.signed_offset_ms(start), 0);
    assert!(win.contains(mid));
    assert!(win.contains(end));
    assert_eq!(win.signed_offset_ms(end), 0);
    assert!(!win.contains(after));
    assert_eq!(win.signed_offset_ms(after), 1);

    let ticks = [
        ("pre", "2026-11-10T22:44:59.999Z"),
        ("t_end", "2026-11-10T22:45:59.999Z"),
        ("t_start", "2026-11-10T22:45:00.000Z"),
        ("t_mid", "2026-11-10T22:45:21.250Z"),
        ("post", "2026-11-10T22:46:00.000Z"),
    ];
    let joined = BiTemporalAudit::join_within_minute_window("2026-11-10 22:45", &ticks)
        .expect("minute join");
    let labels: Vec<&str> = joined.into_iter().map(|(l, _)| l).collect();
    assert_eq!(labels, vec!["t_start", "t_mid", "t_end"]);
}

#[test]
fn value_extraction_ladder_4_rungs_integrity_and_thin_publish_floor() {
    type ParserPassFn = fn(&str) -> Result<String, String>;
    let no_rung4: Option<ParserPassFn> = None;

    // Rung 1: Strict JSON.
    let r1 =
        ValueExtractionLadder::extract_binary_probability(r#"{"probability": 0.42}"#, no_rung4)
            .expect("rung 1");
    assert_eq!(r1.rung, ExtractionRung::Rung1StrictJson);
    assert!((r1.probability - 0.42).abs() < 1e-12);

    // Rung 2: Fenced markdown block.
    let fenced = "Reasoning summary.\n```json\n{\"probability\": 0.63}\n```\n";
    let r2 = ValueExtractionLadder::extract_binary_probability(fenced, no_rung4).expect("rung 2");
    assert_eq!(r2.rung, ExtractionRung::Rung2FencedBlock);
    assert!((r2.probability - 0.63).abs() < 1e-12);

    // Rung 3: Anchor extraction.
    let prose = "Final calibrated estimate: probability: 37.5%";
    let r3 = ValueExtractionLadder::extract_binary_probability(prose, no_rung4).expect("rung 3");
    assert_eq!(r3.rung, ExtractionRung::Rung3AnchorScan);
    assert!((r3.probability - 0.375).abs() < 1e-12);

    // Rung 4: Dedicated parser pass (temperature = 0).
    let unstructured = "roughly three in ten chance overall";
    let r4 = ValueExtractionLadder::extract_binary_probability(
        unstructured,
        Some(|_raw: &str| Ok(r#"{"probability": 0.30}"#.to_owned())),
    )
    .expect("rung 4");
    assert_eq!(r4.rung, ExtractionRung::Rung4ParserPass);
    assert!((r4.probability - 0.30).abs() < 1e-12);

    // Integrity rejections: duplicate keys, booleans, truncated literals.
    let dup = ValueExtractionLadder::extract_binary_probability(
        r#"{"probability": 0.20, "probability": 0.80}"#,
        no_rung4,
    )
    .expect_err("duplicate key rejected");
    assert!(matches!(dup, VerifyError::DuplicateJsonKey(_)));

    let bool_err =
        ValueExtractionLadder::extract_binary_probability(r#"{"probability": true}"#, no_rung4)
            .expect_err("boolean rejected");
    assert!(matches!(bool_err, VerifyError::BooleanAsNumber(_)));

    let trunc =
        ValueExtractionLadder::extract_binary_probability(r#"{"probability": 0.4"#, no_rung4)
            .expect_err("truncated JSON rejected");
    assert!(matches!(trunc, VerifyError::TruncatedOrInvalidLiteral(_)));

    // Option multiset restoration & missing option rejection.
    let canonical = ["10 – 20%", "20 – 30%"];
    let ok_opts = ValueExtractionLadder::extract_option_multiset(
        r#"{"10 - 20%": 0.4, "20 - 30%": 0.6}"#,
        &canonical,
        no_rung4,
    )
    .expect("option names restored");
    assert_eq!(ok_opts.probabilities.get("10 – 20%"), Some(&0.4));
    assert_eq!(ok_opts.probabilities.get("20 – 30%"), Some(&0.6));

    let missing_opt = ValueExtractionLadder::extract_option_multiset(
        r#"{"10 - 20%": 1.0}"#,
        &canonical,
        no_rung4,
    )
    .expect_err("missing option rejected");
    assert!(matches!(
        missing_opt,
        VerifyError::OptionMultisetMismatch(_)
    ));

    // THIN_PUBLISH_FLOOR ([0.05, 0.95] when survivors == 1; [0.02, 0.98] when survivors > 1).
    assert!((apply_publish_floor(0.01, 1) - 0.05).abs() < 1e-12);
    assert!((apply_publish_floor(0.99, 1) - 0.95).abs() < 1e-12);
    assert!((apply_publish_floor(0.01, 3) - 0.02).abs() < 1e-12);
    assert!((apply_publish_floor(0.99, 3) - 0.98).abs() < 1e-12);
}
