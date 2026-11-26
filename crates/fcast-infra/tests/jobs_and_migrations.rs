//! `GATE-02` (0001->0002 migration + V1->V2 upcast) & `§13.2` (`DurableJobStore` + `domain_events`).

use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::forecast::{
    OutcomeDistribution, ProbClamped, ResolutionStatusAtCutoff, UnsealedForecastRevision,
};
use fcast_infra::jobs::{DurableJobStore, JobPriority, JobSpec};
use fcast_infra::store::{OperationalStore, StoreError};
use tempfile::TempDir;

#[test]
fn jobs_priority_coalescing_lease_reclaim_dead_letter_and_budget() {
    let dir = TempDir::new().expect("tempdir");
    let store = OperationalStore::open(dir.path().join("db/ops.fsqlite")).expect("open");
    let jobs = DurableJobStore::new(&store);

    // 1. Coalesced domain events via idempotency_key
    let mut inserted = 0;
    for _ in 0..5 {
        if jobs
            .publish_domain_event(
                "MarketTickShock",
                "q_grid",
                None,
                "2026-11-25T19:45:00Z",
                "shock:q_grid:2026-11-25T19:45",
                "{\"kl\":0.59}",
            )
            .expect("publish")
        {
            inserted += 1;
        }
    }
    assert_eq!(inserted, 1);
    let polled = jobs.poll_domain_events(0).expect("poll");
    assert_eq!(polled.len(), 1);
    assert_eq!(polled[0].event_kind, "MarketTickShock");

    // 2. Idempotent enqueue & P0 > P1 > P2 > P3 priority preemption
    let specs = [
        (
            "sync",
            JobPriority::ScheduledSync,
            "2026-11-26T00:00:00Z",
            1.2,
            0.0,
        ),
        (
            "shock",
            JobPriority::LiveShock,
            "2026-11-25T21:00:00Z",
            8.5,
            0.20,
        ),
        (
            "freeze",
            JobPriority::TournamentFreeze,
            "2026-11-25T20:15:00Z",
            3.1,
            0.25,
        ),
        (
            "redteam",
            JobPriority::RedTeamOrManual,
            "2026-11-25T20:30:00Z",
            4.0,
            0.10,
        ),
    ];
    for (kind, priority, deadline, voi, cost) in specs {
        let (_, is_new) = jobs
            .enqueue_idempotent(&JobSpec {
                kind: kind.into(),
                lane: "epistemic_heavy".into(),
                priority,
                deadline_utc: deadline.into(),
                voi_log: voi,
                idempotency_seed: format!("seed:{kind}"),
                est_cost_usd: cost,
            })
            .expect("enqueue");
        assert!(is_new);
    }
    // Duplicate idempotency_seed rejected cleanly
    let (_, dup_new) = jobs
        .enqueue_idempotent(&JobSpec {
            kind: "freeze".into(),
            lane: "epistemic_heavy".into(),
            priority: JobPriority::TournamentFreeze,
            deadline_utc: "2026-11-25T20:15:00Z".into(),
            voi_log: 3.1,
            idempotency_seed: "seed:freeze".into(),
            est_cost_usd: 0.25,
        })
        .expect("dup enqueue");
    assert!(!dup_new);

    let j1 = jobs
        .dequeue_next("2026-11-25", 1000)
        .expect("j1")
        .expect("some");
    let j2 = jobs
        .dequeue_next("2026-11-25", 1000)
        .expect("j2")
        .expect("some");
    let j3 = jobs
        .dequeue_next("2026-11-25", 1000)
        .expect("j3")
        .expect("some");
    assert_eq!(
        (j1.kind.as_str(), j2.kind.as_str(), j3.kind.as_str()),
        ("freeze", "redteam", "shock")
    );

    // 3. SIGKILL lease recovery reclaims expired RUNNING jobs
    let reclaimed = jobs.reclaim_expired_leases(2000).expect("reclaim");
    assert_eq!(reclaimed, 3);

    // 4. Three consecutive failures quarantine job into DEAD_LETTER
    assert_eq!(
        jobs.record_failure(&j1.job_id, "err1").expect("f1"),
        "QUEUED"
    );
    assert_eq!(
        jobs.record_failure(&j1.job_id, "err2").expect("f2"),
        "QUEUED"
    );
    assert_eq!(
        jobs.record_failure(&j1.job_id, "err3").expect("f3"),
        "DEAD_LETTER"
    );

    // 5. Daily budget circuit breaker blocks metered job when spent + est > cap
    store
        .connection()
        .execute(
            "INSERT INTO daemon_budget_ledger VALUES ('2026-11-25', 4.95, 5.00)",
            [],
        )
        .expect("budget row");
    assert!(matches!(
        jobs.dequeue_next("2026-11-25", 3000),
        Err(StoreError::BudgetCapExceeded { .. })
    ));
}

#[test]
fn gate_02_forward_migration_0002_and_lossless_v1_to_v2_upcast() {
    let dir = TempDir::new().expect("tempdir");
    let store = OperationalStore::open(dir.path().join("db/ops.fsqlite")).expect("open");
    let conn = store.connection();
    conn.execute(
        "INSERT INTO questions VALUES ('q_mig', 'metaculus', 'Mig Test', 'sha', 'point_in_time', '2026-11-01T00:00:00Z')",
        [],
    )
    .expect("q");
    conn.execute(
        "INSERT INTO forecast_streams VALUES ('fc_mig', 'q_mig', 'live_tournament', NULL)",
        [],
    )
    .expect("s");
    conn.execute(
        "INSERT INTO evidence_vault_index VALUES ('[ev:aabb#r0]', 'aabb', 'r0', '2026-11-01T00:00:00Z', 4, 0)",
        [],
    )
    .expect("ev");
    let rev0 = UnsealedForecastRevision {
        forecast_id: "fc_mig".into(),
        seq_no: 0,
        parent_revision_id: None,
        question_id: "q_mig".into(),
        as_of_utc: "2026-11-01T01:00:00Z".into(),
        p_indep: ProbClamped::new(0.30).expect("p"),
        p_market: None,
        probability_distribution: OutcomeDistribution::Binary {
            p: ProbClamped::new(0.30).expect("p"),
            n_eff: 5.0,
        },
        evidence_handles: vec![],
        model_version: "m@v1".into(),
        calibration_version: "cal-v1".into(),
        rationale_handle: VerifiedEvidenceRef::parse("[ev:aabb#r0]").expect("rat"),
        resolution_status: ResolutionStatusAtCutoff::Unresolved,
    }
    .seal()
    .expect("seal");
    store.seal_forecast_revision(&rev0).expect("persist");

    // Pre-migration Zstd-9 backup + 0002 migration + lossless V1 -> V2 upcast read
    let receipt = store
        .apply_migration_0002_with_backup(dir.path().join("backups/ops-pre-v2.fsqlite.zst"))
        .expect("migrate 0002");
    assert!(receipt.backup_path.exists());
    let (ver, p_indep, shift) = store
        .read_revision_upcast_v2(&rev0.revision_id)
        .expect("upcast")
        .expect("found");
    assert_eq!(ver, 1);
    assert!((p_indep - 0.30).abs() < 1e-12);
    assert!((shift - 0.0).abs() < 1e-12);

    // Confirm _fcast_migrations recorded both version 1 and 2
    let cnt: i64 = conn
        .query_row("SELECT COUNT(*) FROM _fcast_migrations", [], |r| r.get(0))
        .expect("count");
    assert_eq!(cnt, 2);
}
