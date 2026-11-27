//! Milestone `M2` + `elicitation-measurement` integration tests:
//! - 4-layer `CausalGraph` DAG acyclicity, `as_of_utc` projection, and 1-hop spillover wakeups
//! - Joint `(model, reasoning_effort, context_tier, rollout_k)` `RepRouter` invariants
//! - Fail-closed `PreLoomGuard` partial-context floor and quantile-downgrade rejection

use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::coherence::ElicitationFormat;
use fcast_epistemic::{
    CausalEdge, CausalGraph, CausalNode, ContextTier, GraphError, GraphLayer, PlanConstraintSpec,
    PreLoomGuard, ReasoningEffort, RepRouter, RepresentationId, TuningCacheSnapshot,
};
use fcast_infra::daemon::{BillingPacingTier, TimeBudget};
use fcast_ingest::{ContextCompleteness, DomainDecision, IntakeSnapshot, TargetDomain};

#[test]
fn m2_causal_graph_enforces_dag_temporal_fence_and_1hop_spillover() {
    let ev1 = VerifiedEvidenceRef::parse("[ev:01234567#kg1]").expect("ev1");
    let ev2 = VerifiedEvidenceRef::parse("[ev:89abcdef#kg2]").expect("ev2");
    let ev3 = VerifiedEvidenceRef::parse("[ev:fedcba98#kg3]").expect("ev3");

    let mut g = CausalGraph::new();
    g.add_node(CausalNode {
        node_id: "n_hormuz".into(),
        layer: GraphLayer::L0Verified,
        label: "Strait of Hormuz closure".into(),
        question_id: Some("q_hormuz".into()),
        ev_handle: ev1.clone(),
        available_utc: "2026-10-10T00:00:00Z".into(),
    })
    .expect("n1");
    g.add_node(CausalNode {
        node_id: "n_brent".into(),
        layer: GraphLayer::L1Mechanism,
        label: "Brent crude > $100".into(),
        question_id: Some("q_brent".into()),
        ev_handle: ev2.clone(),
        available_utc: "2026-10-10T00:00:00Z".into(),
    })
    .expect("n2");
    g.add_node(CausalNode {
        node_id: "n_future".into(),
        layer: GraphLayer::L2Signpost,
        label: "Post-cutoff tanker reroute".into(),
        question_id: Some("q_freight".into()),
        ev_handle: ev3.clone(),
        available_utc: "2026-10-16T00:00:00Z".into(),
    })
    .expect("n3");

    g.add_edge(CausalEdge {
        from_id: "n_hormuz".into(),
        to_id: "n_brent".into(),
        layer: GraphLayer::L1Mechanism,
        polarity: 1,
        strength: 0.80,
        ev_handle: ev2.clone(),
        available_utc: "2026-10-10T00:00:00Z".into(),
    })
    .expect("e1");
    g.add_edge(CausalEdge {
        from_id: "n_hormuz".into(),
        to_id: "n_future".into(),
        layer: GraphLayer::L2Signpost,
        polarity: 1,
        strength: 0.60,
        ev_handle: ev3.clone(),
        available_utc: "2026-10-16T00:00:00Z".into(),
    })
    .expect("e2");

    // DAG cycle on L1Mechanism MUST be rejected
    assert!(matches!(
        g.add_edge(CausalEdge {
            from_id: "n_brent".into(),
            to_id: "n_hormuz".into(),
            layer: GraphLayer::L1Mechanism,
            polarity: 1,
            strength: 0.50,
            ev_handle: ev2,
            available_utc: "2026-10-10T00:00:00Z".into(),
        }),
        Err(GraphError::CycleDetected { .. })
    ));

    // Temporal projection at 2026-10-15 excludes n_future and e2 (0 future leaks!)
    let fenced = g
        .project_as_of("2026-10-15T00:00:00Z")
        .expect("project_as_of");
    assert_eq!(fenced.node_count(), 2);
    assert_eq!(fenced.edge_count(), 1);

    // 1-hop spillover from q_hormuz (+0.50 logit) wakes q_brent (+0.40 logit)
    let wakeups = fenced.spillover_targets("q_hormuz", 0.50, 0.25);
    assert_eq!(wakeups.len(), 1);
    assert_eq!(wakeups[0].target_question_id, "q_brent");
    assert!((wakeups[0].propagated_delta_logit - 0.40).abs() < 1e-12);
}

#[test]
fn elicitation_rep_router_never_pairs_complete_primary_with_default_effort() {
    let snap = TuningCacheSnapshot::cold_start();
    assert!(!snap.joint_cells.is_empty());

    let intake = IntakeSnapshot {
        question_id: "q_bill_s3596".into(),
        platform_id: "metaculus".into(),
        question_type: "numeric".into(),
        domain_decision: DomainDecision::Accept {
            domain: TargetDomain::Macroeconomics,
            confidence: 0.95,
            matched_signals: vec!["cpi".into()],
        },
        book_mark: None,
        complexity_score: 80.0,
        prize_weight: 2.0,
    };
    let spec = PlanConstraintSpec {
        soft_max_cost_usd: 0.15,
        soft_max_latency_sec: 60.0,
        accepted_override_rep: Some(RepresentationId::R3ThreeDocOverlay),
        autonomous_relax_factor: 0.50,
    };
    let tb_full = TimeBudget::from_epochs(10_000, Some(15_000), 20_000);
    let tb_fast = TimeBudget::from_epochs(10_000, Some(10_300), 20_000);

    // Full + R3 -> Max effort + CompletePrimary + median-of-5 rollouts + fetch_raw_data=true
    let full_plan = RepRouter::phase_b_select(
        &intake,
        "2026-10-15T00:00:00Z",
        &spec,
        &tb_full,
        BillingPacingTier::Full,
        &snap,
    );
    assert_eq!(full_plan.reasoning_effort, ReasoningEffort::Max);
    assert_eq!(full_plan.context_tier, ContextTier::CompletePrimary);
    assert_eq!(full_plan.rollout_k, 5);
    assert!(full_plan.evidence_plan.as_ref().expect("ep").fetch_raw_data);

    // Economy + R3 -> drops heavy raw text to SummaryOnly + Medium effort (NEVER CompletePrimary at Default!)
    let econ_plan = RepRouter::phase_b_select(
        &intake,
        "2026-10-15T00:00:00Z",
        &spec,
        &tb_full,
        BillingPacingTier::Economy,
        &snap,
    );
    assert_eq!(econ_plan.reasoning_effort, ReasoningEffort::Medium);
    assert_eq!(econ_plan.context_tier, ContextTier::SummaryOnly);
    assert_eq!(econ_plan.rollout_k, 3);
    assert!(!econ_plan.evidence_plan.as_ref().expect("ep").fetch_raw_data);

    // FastPath -> Default effort + Minimal context + k=1
    let fast_plan = RepRouter::phase_b_select(
        &intake,
        "2026-10-15T00:00:00Z",
        &spec,
        &tb_fast,
        BillingPacingTier::Full,
        &snap,
    );
    assert_eq!(fast_plan.reasoning_effort, ReasoningEffort::Default);
    assert_eq!(fast_plan.context_tier, ContextTier::Minimal);
    assert_eq!(fast_plan.rollout_k, 1);

    // PreLoomGuard: partial context forces THIN_PUBLISH_FLOOR [0.05, 0.95] even with 3 survivors
    let (p_partial, _) = PreLoomGuard::extract_and_guard_with_completeness(
        "{\"probability\": 0.99}",
        3,
        &ContextCompleteness::Partial {
            reason: "truncated statutory appendix".into(),
        },
    )
    .expect("extract");
    assert!((p_partial.get() - 0.95).abs() < 1e-12);

    // PreLoomGuard: fail-closed on silent quantile downgrade & zero survivors
    assert!(
        PreLoomGuard::validate_quantile_promotion(
            ElicitationFormat::NativeQuantiles,
            ElicitationFormat::IntervalTransform3Pt
        )
        .is_err()
    );
    let (p0, _) = PreLoomGuard::extract_and_guard_prob("{\"probability\": 0.99}", 0).expect("p0");
    assert!(p0.get() <= 0.95 + 1e-12);

    // JSON roundtrip for TuningCacheSnapshot (regression test for struct-key serialization bug)
    let snap_json = serde_json::to_string(&snap).expect("serialize snapshot");
    let snap_rt: TuningCacheSnapshot = serde_json::from_str(&snap_json).expect("deserialize");
    assert_eq!(snap, snap_rt);

    // NaN cell error_loss must NEVER win selection, and tight cost cap reports no_feasible_representation
    let mut bad_snap = TuningCacheSnapshot::cold_start();
    bad_snap.joint_cells.clear();
    if let Some(c) = bad_snap.cells.get_mut(&RepresentationId::R4CausalGraphFold) {
        c.mean_error_loss = f64::NAN;
    }
    let safe_plan = RepRouter::phase_b_select(
        &intake,
        "2026-10-15T00:00:00Z",
        &spec,
        &tb_full,
        BillingPacingTier::Full,
        &bad_snap,
    );
    assert_ne!(
        safe_plan.selected_rep,
        Some(RepresentationId::R4CausalGraphFold)
    );

    let tight = PlanConstraintSpec {
        soft_max_cost_usd: 0.001,
        ..Default::default()
    };
    let tight_plan = RepRouter::phase_b_select(
        &intake,
        "2026-10-15T00:00:00Z",
        &tight,
        &tb_full,
        BillingPacingTier::Full,
        &snap,
    );
    assert!(tight_plan.skipped);
    assert_eq!(
        tight_plan.skip_reason.as_deref(),
        Some("no_feasible_representation")
    );

    // PostProposalAudit must block confirmed G1 / unverified claims even on small logit shifts (<0.35)
    let small_g1 = fcast_epistemic::DraftRevisionProposal {
        question_id: "q_salami".into(),
        p_prev: fcast_core::forecast::ProbClamped::new(0.50).expect("p"),
        p_proposed: fcast_core::forecast::ProbClamped::new(0.54).expect("p"),
        is_freeze_deadline: false,
        contract_misread_g1: true,
        unverified_load_bearing_claims: vec!["unverified_claim".into()],
    };
    assert!(matches!(
        fcast_epistemic::PostProposalAudit::evaluate_binding(&small_g1),
        fcast_epistemic::BindingAuditVerdict::BlockPublish { .. }
    ));
}
