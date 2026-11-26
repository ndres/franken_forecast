//! End-to-end verification of all 4 MIT Axiomatic Design lower-triangular splits:
//! 1. `FR3a` (`IntakeSnapshot`) -> `FR4` (`RepRouter` Phase A & B) -> `FR3b` (`EvidenceAcquisitionPlan` -> `EvidenceBundle`).
//! 2. `FR4` reads ONLY immutable `TuningCacheSnapshot(t-1)` (`0005_tuning_cache_snapshots.sql`) + external `PlanConstraintSpec` (LCB cold-start protection).
//! 3. `FR6a` (`PreLoomGuard` stateless extraction + `THIN_PUBLISH_FLOOR`) BEFORE `Loom` -> `FR6b` (`PostProposalAudit` binding verdict) AFTER `Loom` draft.
//! 4. `FR7` (`kalman_market_blend` using `mean_shift_surprise_nats`) schedules *next-timestep `t+1`* `MarketTickShock` without mutating step-`t` `p_indep`.

use fcast_core::belief::kalman_market_blend;
use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::forecast::ProbClamped;
use fcast_epistemic::{
    BindingAuditVerdict, DraftRevisionProposal, PlanConstraintSpec, PostProposalAudit,
    PreLoomGuard, RepRouter, RepresentationId, TuningCacheSnapshot,
};
use fcast_infra::daemon::{BillingPacingTier, TimeBudget};
use fcast_infra::jobs::DurableJobStore;
use fcast_infra::store::OperationalStore;
use fcast_ingest::{
    BookMarkPrice, DomainDecision, IntakeSnapshot, OrderbookSnapshot, TargetDomain,
    assemble_evidence_bundle,
};
use tempfile::TempDir;

#[test]
fn all_four_axiomatic_lower_triangular_splits_work_end_to_end() {
    let dir = TempDir::new().expect("tempdir");
    let store = OperationalStore::open(dir.path().join("db/ops.fsqlite")).expect("open");

    // --- Split 2: Persist immutable TuningCacheSnapshot(t-1) in Chronicle ---
    let cold_snap = TuningCacheSnapshot::cold_start();
    let snap_json = serde_json::to_string(&cold_snap).expect("serialize snap");
    store
        .save_tuning_snapshot(
            &cold_snap.snapshot_id,
            &cold_snap.created_utc,
            cold_snap.resolved_n,
            &snap_json,
        )
        .expect("save tuning snapshot");
    // SQLite trigger MUST block mutating or deleting a persisted TuningCacheSnapshot
    assert!(
        store
            .connection()
            .execute(
                "UPDATE rep_router_tuning_snapshots SET resolved_n = 999 WHERE snapshot_id = ?1",
                [&cold_snap.snapshot_id],
            )
            .is_err()
    );
    let (_, _, _, loaded_json) = store
        .load_latest_tuning_snapshot()
        .expect("load")
        .expect("some");
    let loaded_snap: TuningCacheSnapshot =
        serde_json::from_str(&loaded_json).expect("deserialize snap");

    // --- Split 1 (FR3a): Contract & Mark Intake BEFORE RepRouter ---
    let snap_ob = OrderbookSnapshot::new(0.44, 450.0, 0.46, 550.0, 0.01);
    let mark = BookMarkPrice::from_snapshot(&snap_ob).expect("mark");
    let intake = IntakeSnapshot {
        question_id: "q_eu_grid_m15".into(),
        platform_id: "metaculus".into(),
        question_type: "binary".into(),
        domain_decision: DomainDecision::Accept {
            domain: TargetDomain::Cybersecurity,
            confidence: 0.95,
            matched_signals: vec!["grid".into()],
        },
        book_mark: Some(mark),
        complexity_score: 72.0,
        prize_weight: 2.0,
    };

    // --- Split 2 (FR4): Two-Phase ReSHAPE Forecasting Planning over immutable snapshot(t-1) ---
    let time_budget = TimeBudget::from_epochs(10_000, Some(15_000), 20_000);
    let spec = PlanConstraintSpec {
        soft_max_cost_usd: 0.10, // R3 ($0.11) slightly exceeds soft cap -> Phase A surfaces it!
        soft_max_latency_sec: 60.0,
        accepted_override_rep: None,
        autonomous_relax_factor: 0.50,
    };
    let phase_a = RepRouter::phase_a_reshape(
        &intake,
        &spec,
        &time_budget,
        BillingPacingTier::Full,
        &loaded_snap,
    );
    assert!(
        phase_a.iter().any(|c| c.relaxes_soft_constraint),
        "Phase A must surface concrete soft-relaxed boundary-probing plan"
    );
    // Cold-start LCB prevents collapsing onto cheapest R0 when R3 has stronger LCB!
    let selected = RepRouter::phase_b_select(
        &intake,
        "2026-11-26T12:00:00Z",
        &spec,
        &time_budget,
        BillingPacingTier::Full,
        &loaded_snap,
    );
    assert!(!selected.skipped);
    assert_eq!(
        selected.selected_rep,
        Some(RepresentationId::R3ThreeDocOverlay)
    );
    let ev_plan = selected.evidence_plan.expect("FR3b evidence plan");
    assert!(
        ev_plan.fetch_raw_data && ev_plan.attach_seasonal_note && ev_plan.dual_independent_reports
    );

    // --- Split 1 (FR3b): Plan-Driven Evidence Acquisition strictly AFTER RepRouter ---
    let bundle = assemble_evidence_bundle(
        &ev_plan,
        vec![VerifiedEvidenceRef::parse("[ev:deadbeef#raw1]").expect("ref")],
    );
    assert_eq!(bundle.report_count, 2);
    assert!(bundle.raw_data_attached && bundle.seasonal_note_attached);

    // --- Split 3 (FR6a): PreLoomGuard stateless extraction + THIN_PUBLISH_FLOOR BEFORE Loom ---
    let (p_guarded, _) =
        PreLoomGuard::extract_and_guard_prob(r#"{"probability": 0.99}"#, 1).expect("extract");
    assert!((p_guarded.get() - 0.95).abs() < 1e-12);

    // --- Split 3 (FR6b): PostProposalAudit binding verdict AFTER Loom draft ---
    let blocked = PostProposalAudit::evaluate_binding(&DraftRevisionProposal {
        question_id: "q_eu_grid_m15".into(),
        p_prev: ProbClamped::new(0.40).expect("p"),
        p_proposed: ProbClamped::new(0.75).expect("p"),
        is_freeze_deadline: false,
        contract_misread_g1: true,
        unverified_load_bearing_claims: vec![],
    });
    assert!(matches!(blocked, BindingAuditVerdict::BlockPublish { .. }));

    let approved = PostProposalAudit::evaluate_binding(&DraftRevisionProposal {
        question_id: "q_eu_grid_m15".into(),
        p_prev: ProbClamped::new(0.40).expect("p"),
        p_proposed: ProbClamped::new(0.55).expect("p"),
        is_freeze_deadline: false,
        contract_misread_g1: false,
        unverified_load_bearing_claims: vec![],
    });
    assert!(matches!(approved, BindingAuditVerdict::Approve { .. }));

    // --- Split 4 (FR7 -> FR2 t+1): Kalman mean_shift_surprise_nats schedules next-step shock only ---
    let jobs = DurableJobStore::new(&store);
    // Exact agreement on deep book -> mean_shift_surprise_nats == 0.0 -> ZERO spurious wakeups!
    let agree = kalman_market_blend(
        ProbClamped::new(0.45).expect("p"),
        0.25,
        ProbClamped::new(0.45).expect("p"),
        0.90,
        0.10,
    );
    assert!(
        !jobs
            .schedule_next_step_market_shock(
                "q_eu_grid_m15",
                "2026-11-26T12:00:00Z",
                "2026-11-26T13:00:00Z",
                &agree,
                0.15,
            )
            .expect("shock check agree")
    );

    // Large genuine market disagreement on deep book -> enqueues t+1 LiveShock job cleanly
    let disagree = kalman_market_blend(
        ProbClamped::new(0.20).expect("p"),
        0.25,
        ProbClamped::new(0.55).expect("p"),
        0.90,
        0.10,
    );
    assert!(
        jobs.schedule_next_step_market_shock(
            "q_eu_grid_m15",
            "2026-11-26T12:01:00Z",
            "2026-11-26T13:00:00Z",
            &disagree,
            0.15,
        )
        .expect("shock check disagree")
    );
    let events = jobs.poll_domain_events(0).expect("poll");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "MarketTickShock");
}
