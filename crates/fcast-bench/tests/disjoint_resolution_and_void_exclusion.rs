//! `GATE-06` (`CLAIM-02`, `SUB-05`): Disjoint `QuestionResolution` never mutates
//! `forecast_revisions.resolution_status`, `invalidated` outcomes cleanly drop from Brier/Log
//! denominators, Thesis Lab `1e-12` relative dispersion floor catches linear floating-point
//! residue, and the `M1.5` demo proves full-universe Arm B (`R3`) lift over Arm A (`R0`).

use fcast_bench::{
    compute_history_step_scale, deterministic_m15_demo_cohort, paired_normalized_crps_delta,
    run_m15_demo_pipeline,
};
use fcast_infra::store::OperationalStore;
use fcast_infra::vault::EvidenceVault;
use tempfile::TempDir;

#[test]
fn gate_06_disjoint_resolution_void_exclusion_and_m15_skill_demo() {
    let dir = TempDir::new().expect("tempdir");
    let store = OperationalStore::open(dir.path().join("db/ops.fsqlite")).expect("open");
    let vault = EvidenceVault::open(dir.path().join("vault")).expect("vault");
    let cohort = deterministic_m15_demo_cohort();

    let report = run_m15_demo_pipeline(&store, &vault, &cohort, None).expect("run m15 demo");
    assert_eq!(report.cohort_size, 12);
    assert_eq!(report.temporal_leaks, 0);

    // Void question (m15_void_ambiguous) MUST be excluded from Brier/Log denominators (10 resolved issued, 11 resolved eligible incl. 1 abandoned)
    assert_eq!(report.arm_r3.void_excluded_total, 1);
    assert_eq!(report.arm_r3.abandoned_total, 1);
    assert_eq!(report.arm_r3.resolved_issued_count, 10);
    assert_eq!(report.arm_r3.resolved_eligible_count, 11);

    // INV-01 & GATE-06: Every single ForecastRevision row MUST still have resolution_status == 'unresolved'
    let mutated_rows: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM forecast_revisions WHERE resolution_status != 'unresolved'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(
        mutated_rows, 0,
        "QuestionResolution must NEVER mutate forecast_revisions"
    );

    // Full-universe anti-selection-bias proof: sunk abandoned cost makes cost/resolved > cost/issued
    assert!(report.arm_r3.cost_per_resolved_usd_incl_sunk > report.arm_r3.cost_per_issued_usd);

    // Single-knob skill gate: Arm B (R3 structured + Red-Team) beats Arm A (R0) and crowd baseline
    assert!(report.skill_gate_passed);
    assert!(report.delta_brier_full_universe < -0.02);
    assert!(report.delta_log_full_universe > 0.05);
    assert!(report.arm_r3.bss_vs_crowd_full_universe > 0.25);
}

#[test]
fn thesis_lab_relative_dispersion_floor_rejects_linear_fp_residue_and_scores_crps_properly() {
    // Exact Thesis Lab scar (docs/brier-lab.md): linear decimal history has ~1.57e-16 float residue!
    let linear_fp_residue = [1.814, 1.805, 1.796];
    assert!(
        compute_history_step_scale(&linear_fp_residue).is_none(),
        "1e-12 relative dispersion floor must reject linear floating-point residue"
    );
    assert!(paired_normalized_crps_delta(1.787, 0.01, 1.787, &linear_fp_residue).is_none());

    // Real non-degenerate history: accurate forecast beats persistence (delta < 0.0)
    let real_hist = [4.1, 4.2, 4.0, 4.3, 4.1, 4.2];
    let scale = compute_history_step_scale(&real_hist).expect("usable scale");
    assert!(scale > 0.1);
    let delta_good = paired_normalized_crps_delta(4.48, 0.12, 4.50, &real_hist).expect("delta");
    let delta_bad = paired_normalized_crps_delta(3.90, 0.12, 4.50, &real_hist).expect("delta");
    assert!(
        delta_good < 0.0,
        "accurate forecast should beat persistence: {delta_good}"
    );
    assert!(
        delta_bad > 0.0,
        "wrong-direction forecast should lose to persistence: {delta_bad}"
    );
}
