//! `GATE-16` integration test: Staged-reveal price isolation (`INV-05`).
//!
//! Proves that the Turn A wire JSON payload contains `0` occurrences of `forecaster_forecast`
//! or `crowd_prior`, passes `MarketBlindingScrubber`, and that Turn B enforces locked Turn A
//! integer immutability and 3-tier `crowd_prior` resolution.

use std::path::PathBuf;

use fcast_core::blinding::{MarketBlindingScrubber, VerifiedEvidenceRef};
use fcast_core::forecast::{OutcomeDistribution, ProbClamped};
use fcast_epistemic::{
    CrowdPriorSource, ForecastBenchMedianRow, LockedTurnAScores, RationaleError, TemplateCatalog,
    TurnARequest, TurnBRequest, TurnBResponse, resolve_crowd_prior, score_rationale,
};

fn workspace_templates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("templates")
}

#[test]
fn turn_a_wire_json_has_zero_forecaster_forecast_or_crowd_prior_occurrences() {
    let req = TurnARequest {
        question_id: "q_red_sea_2026".to_owned(),
        as_of_utc: "2026-11-18T00:00:00Z".to_owned(),
        contract_sha256: "sha256_red_sea_v1".to_owned(),
        resolution_criteria: "Resolves YES if Bab el-Mandeb transits drop >50% YoY.".to_owned(),
        background_as_of_utc: "Container rerouting around Cape of Good Hope elevated.".to_owned(),
        rationale_handle: VerifiedEvidenceRef::parse("[ev:deadbeef#rat1]").expect("handle"),
        rationale_text: "Base rate for chokepoint disruptions lasting >90 days is 18% (4/22). \
                         Contract requires >50% YoY drop per IMF PortWatch."
            .to_owned(),
    };

    let wire_json = req.to_wire_json().expect("serialize Turn A");
    assert_eq!(
        wire_json.matches("forecaster_forecast").count(),
        0,
        "Turn A wire JSON leaked forecaster_forecast"
    );
    assert_eq!(
        wire_json.matches("crowd_prior").count(),
        0,
        "Turn A wire JSON leaked crowd_prior"
    );
    assert_eq!(
        wire_json.matches("p_market").count(),
        0,
        "Turn A wire JSON leaked p_market"
    );
    MarketBlindingScrubber::assert_blind_payload(&wire_json)
        .expect("Turn A wire JSON must pass MarketBlindingScrubber");

    let catalog = TemplateCatalog::load_and_verify(&workspace_templates_dir()).expect("catalog");
    let rendered = req.render_prompt(&catalog).expect("render Turn A prompt");
    assert!(rendered.contains("q_red_sea_2026"));
    assert!(!rendered.contains("{p_market}"));
    assert!(!rendered.contains("{forecaster_probability_distribution}"));
}

#[test]
fn turn_b_reveals_pricing_and_blocks_turn_a_lock_tampering() {
    let fb_row = ForecastBenchMedianRow {
        freeze_crowd_median: Some(0.34),
        sf_median: Some(0.31),
        public_median: Some(0.38),
        available_utc: "2026-11-15T00:00:00Z".to_owned(),
    };

    // 3-tier crowd_prior cascade checks:
    // Pre-seal blind mode always returns None.
    assert_eq!(
        resolve_crowd_prior(
            Some(0.39),
            false,
            Some(&fb_row),
            Some(0.36),
            "2026-11-18T00:00:00Z",
            true
        ),
        None
    );
    // Tier 1: live liquid venue.
    assert_eq!(
        resolve_crowd_prior(
            Some(0.39),
            false,
            Some(&fb_row),
            Some(0.36),
            "2026-11-18T00:00:00Z",
            false
        ),
        Some((CrowdPriorSource::VenueLive, 0.39))
    );
    // Tier 2: masked venue falls back to ForecastBench published median (available_utc <= as_of_utc).
    let resolved_prior = resolve_crowd_prior(
        None,
        true,
        Some(&fb_row),
        Some(0.36),
        "2026-11-18T00:00:00Z",
        false,
    );
    assert_eq!(
        resolved_prior,
        Some((CrowdPriorSource::ForecastBenchMedian, 0.34))
    );
    // Post-cutoff ForecastBench row is fenced off -> falls back to Tier 3 cluster consensus.
    let late_fb = ForecastBenchMedianRow {
        available_utc: "2026-11-25T00:00:00Z".to_owned(),
        ..fb_row
    };
    assert_eq!(
        resolve_crowd_prior(
            None,
            true,
            Some(&late_fb),
            Some(0.36),
            "2026-11-18T00:00:00Z",
            false
        ),
        Some((CrowdPriorSource::ClusterConsensus, 0.36))
    );

    let locked = LockedTurnAScores {
        question_id: "q_red_sea_2026".to_owned(),
        rationale_handle: VerifiedEvidenceRef::parse("[ev:deadbeef#rat1]").expect("handle"),
        as_of_utc: "2026-11-18T00:00:00Z".to_owned(),
        ct: Some(4),
        ei: Some(3),
        ov: Some(4),
        me: Some(3),
        ej: Some(3),
        unscoreable: false,
    };

    let turn_b = TurnBRequest::new(
        "inter_turn_a_001".to_owned(),
        locked,
        "2026-11-18T00:00:00Z".to_owned(),
        OutcomeDistribution::Binary {
            p: ProbClamped::new(0.28).expect("prob"),
            n_eff: 10.0,
        },
        resolved_prior,
    )
    .expect("Turn B request");

    let wire_b = turn_b.to_wire_json().expect("serialize Turn B");
    assert!(wire_b.contains("forecaster_forecast"));
    assert!(wire_b.contains("crowd_prior"));

    // Tampered Turn B response altering locked `ov` from 4 -> 2 must be rejected.
    let tampered = TurnBResponse {
        ct: Some(4),
        ei: Some(3),
        ov: Some(2),
        me: Some(3),
        ej: Some(3),
        pr: Some(3),
        g1: false,
        g2: false,
        unscoreable: false,
        note: "Coherent 6pp divergence from 34% crowd prior.".to_owned(),
    };
    let tamper_err = turn_b
        .finalize_record(
            "rat_1".to_owned(),
            "2026-11-18T00:01:00Z".to_owned(),
            tampered,
        )
        .expect_err("tampered Turn A integer must be rejected");
    assert_eq!(
        tamper_err,
        RationaleError::TurnALockTampered {
            dim: "ov",
            expected: Some(4),
            actual: Some(2),
        }
    );

    // Faithful echo succeeds and scores cleanly in Rust (INV-06).
    let faithful = TurnBResponse {
        ct: Some(4),
        ei: Some(3),
        ov: Some(4),
        me: Some(3),
        ej: Some(3),
        pr: Some(4),
        g1: false,
        g2: false,
        unscoreable: false,
        note: "Coherent 6pp divergence from 34% crowd prior grounded in PortWatch base rate."
            .to_owned(),
    };
    let rec = turn_b
        .finalize_record(
            "rat_1".to_owned(),
            "2026-11-18T00:01:00Z".to_owned(),
            faithful,
        )
        .expect("finalize record");
    let scored = score_rationale(&rec).expect("score").expect("scoreable");
    assert_eq!(scored.effective_score, 21);
}
