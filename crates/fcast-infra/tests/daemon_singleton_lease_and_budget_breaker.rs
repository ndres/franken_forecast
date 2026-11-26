//! `GATE-19` (`CLAIM-07`, `SUB-06`): singleton `DaemonLock`, expired lease reclaim, `BLAKE3`
//! idempotency, `$5.00/day` budget circuit breaker, `0600` `UdsEventBus`, durable `event_consumers`
//! crash replay, close-derived `TimeBudget`, 4-tier `BillingPacingTier`, `WatchdogAction`,
//! and `scrub_secrets`.

use fcast_infra::daemon::{
    AbSlot, BillingPacingTier, DaemonBusEvent, FAST_PATH_THRESHOLD_SEC, MIN_INTAKE_SEC,
    QuestionRunContext, TimeBudget, TimeBudgetMode, UdsEventBus, WATCHDOG_ALERT_SEC,
    WATCHDOG_KILL_RECLAIM_SEC, WatchdogAction, scrub_secrets,
};
use fcast_infra::jobs::{DurableJobStore, JobPriority, JobSpec};
use fcast_infra::store::{OperationalStore, StoreError};
use fcast_infra::sys::{
    DaemonLock, DaemonLockError, DataRootLayout, MAX_UDS_SUN_PATH_BYTES, UdsPathError,
    validate_uds_sun_path,
};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn gate_19_daemon_singleton_lock_and_sun_path_ceiling() {
    let dir = TempDir::new().expect("tempdir");
    let layout = DataRootLayout::init(dir.path()).expect("init layout");
    let lock_path = layout.daemon_lock_path();

    // First fcastd instance acquires exclusive non-blocking flock
    let lock1 = DaemonLock::acquire(&lock_path).expect("acquire first lock");
    assert_eq!(lock1.path(), lock_path.as_path());

    // Duplicate fcastd instance is rejected immediately with AlreadyLocked
    let dup = DaemonLock::acquire(&lock_path);
    assert!(matches!(dup, Err(DaemonLockError::AlreadyLocked { .. })));

    // Dropping first instance releases lock cleanly so a restarted daemon can re-acquire
    drop(lock1);
    let lock2 = DaemonLock::acquire(&lock_path).expect("re-acquire after drop");
    drop(lock2);

    // POSIX sun_path <= 104-byte validation
    assert_eq!(validate_uds_sun_path(""), Err(UdsPathError::EmptyPath));
    let exact_104 = format!("/{}", "a".repeat(MAX_UDS_SUN_PATH_BYTES - 1));
    assert_eq!(exact_104.len(), MAX_UDS_SUN_PATH_BYTES);
    assert!(validate_uds_sun_path(&exact_104).is_ok());

    let over_104 = format!("/{}", "a".repeat(MAX_UDS_SUN_PATH_BYTES));
    assert_eq!(over_104.len(), MAX_UDS_SUN_PATH_BYTES + 1);
    assert!(matches!(
        validate_uds_sun_path(&over_104),
        Err(UdsPathError::PathTooLong {
            byte_len: 105,
            max_bytes: 104,
            ..
        })
    ));
}

#[test]
fn gate_19_lease_reclaim_blake3_idempotency_and_daily_budget_breaker() {
    let dir = TempDir::new().expect("tempdir");
    let store = OperationalStore::open(dir.path().join("db/ops.fsqlite")).expect("open");
    let jobs = DurableJobStore::new(&store);

    let spec = JobSpec {
        kind: "belief_fold".into(),
        lane: "epistemic_heavy".into(),
        priority: JobPriority::TournamentFreeze,
        deadline_utc: "2026-11-26T12:00:00Z".into(),
        voi_log: 6.4,
        idempotency_seed: "fold:q_eu_grid:2026-11-26T11:00Z".into(),
        est_cost_usd: 0.40,
    };

    // BLAKE3 deterministic job_id + idempotent deduplication
    let expected_prefix = format!(
        "job_{}",
        &blake3::hash(spec.idempotency_seed.as_bytes()).to_hex()[0..16]
    );
    let (job_id, inserted) = jobs.enqueue_idempotent(&spec).expect("enqueue 1");
    assert!(inserted);
    assert_eq!(job_id, expected_prefix);

    let (dup_id, dup_inserted) = jobs.enqueue_idempotent(&spec).expect("enqueue 2");
    assert!(!dup_inserted);
    assert_eq!(dup_id, job_id);

    // Dequeue acquires a visibility lease; simulated SIGKILL leaves it RUNNING past expiry
    let running = jobs
        .dequeue_next("2026-11-26", 1_000)
        .expect("dequeue")
        .expect("job");
    assert_eq!(running.job_id, job_id);
    assert!(
        jobs.dequeue_next("2026-11-26", 1_050)
            .expect("empty while leased")
            .is_none()
    );

    // Reclaim before expiry reclaims 0; reclaim after expiry reclaims 1
    assert_eq!(jobs.reclaim_expired_leases(1_000).expect("not expired"), 0);
    assert_eq!(jobs.reclaim_expired_leases(1_001).expect("expired"), 1);

    // $5.00/day budget circuit breaker blocks metered dispatch once spent + est > 5.00
    store
        .connection()
        .execute(
            "INSERT INTO daemon_budget_ledger (utc_day, spent_usd, cap_usd) VALUES ('2026-11-26', 4.70, 5.00)",
            [],
        )
        .expect("insert ledger");
    let err = jobs.dequeue_next("2026-11-26", 2_000);
    assert!(matches!(
        err,
        Err(StoreError::BudgetCapExceeded {
            spent_usd,
            est_usd,
            cap_usd,
        }) if (spent_usd - 4.70).abs() < 1e-9
            && (est_usd - 0.40).abs() < 1e-9
            && (cap_usd - 5.00).abs() < 1e-9
    ));
}

#[test]
fn gate_19_uds_event_bus_0600_roundtrip_and_cleanup() {
    let dir = tempfile::Builder::new()
        .prefix("fc")
        .tempdir_in("/tmp")
        .expect("short tempdir in /tmp");
    let sock_path = dir.path().join("daemon.sock");
    validate_uds_sun_path(&sock_path).expect("short sock path fits sun_path");

    let bus = UdsEventBus::bind(&sock_path).expect("bind UDS bus");
    let mode = std::fs::metadata(&sock_path)
        .expect("stat sock")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "daemon.sock must be 0600");

    let expected_event = DaemonBusEvent {
        event_seq: 42,
        event_kind: "RevisionSealed".into(),
        question_id: "q_eu_grid".into(),
        forecast_id: Some("fc_eu_grid".into()),
        as_of_utc: "2026-11-26T12:00:00Z".into(),
        payload_json: "{\"p_indep\":0.24}".into(),
    };

    // Kernel listen queue accepts connect() on the same thread with zero deadlock risk.
    let mut client_stream = UdsEventBus::connect(&sock_path).expect("connect");
    let mut server_stream = bus.accept().expect("accept");
    UdsEventBus::push_event(&mut server_stream, &expected_event).expect("push");
    let received = UdsEventBus::recv_event(&mut client_stream).expect("recv");
    assert_eq!(received, expected_event);

    drop(bus);
    assert!(
        !sock_path.exists(),
        "dropping UdsEventBus must clean up daemon.sock"
    );

    // Binding over >104-byte path fails cleanly before touching filesystem
    let too_long = PathBuf::from(format!("/tmp/{}/daemon.sock", "x".repeat(110)));
    assert!(UdsEventBus::bind(&too_long).is_err());
}

#[test]
fn gate_19_durable_event_consumers_crash_replay_and_migration_0003() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("db/ops.fsqlite");

    // First process lifecycle: apply migration 0003, publish 3 events, ack only #1 for consumer A
    {
        let store = OperationalStore::open(&db_path).expect("open 1");
        store.apply_migration_0003().expect("migration 0003");
        // Idempotent second call
        store.apply_migration_0003().expect("migration 0003 idem");

        let has_v3: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM _fcast_migrations WHERE version = 3 AND name = '0003_event_consumers'",
                [],
                |r| r.get(0),
            )
            .expect("v3 check");
        assert_eq!(has_v3, 1);

        let jobs = DurableJobStore::new(&store);
        assert!(
            jobs.register_consumer("loom_belief", "2026-11-26T10:00:00Z")
                .expect("reg loom")
        );
        assert!(
            !jobs
                .register_consumer("loom_belief", "2026-11-26T10:00:01Z")
                .expect("reg loom dup")
        );
        assert!(
            jobs.register_consumer("cockpit_push", "2026-11-26T10:00:00Z")
                .expect("reg cockpit")
        );

        for idx in 1..=3 {
            let inserted = jobs
                .publish_domain_event(
                    "MarketTickShock",
                    "q_hormuz",
                    None,
                    &format!("2026-11-26T10:0{idx}:00Z"),
                    &format!("shock:q_hormuz:min{idx}"),
                    &format!("{{\"idx\":{idx}}}"),
                )
                .expect("publish");
            assert!(inserted);
        }

        let initial = jobs
            .replay_unacked_events("loom_belief")
            .expect("replay loom");
        assert_eq!(initial.len(), 3);
        let first_seq = initial[0].event_seq;
        let second_seq = initial[1].event_seq;
        assert!(first_seq < second_seq);

        // Ack event #2, then verify out-of-order older ack (#1) does NOT regress cursor
        assert_eq!(
            jobs.ack_consumer_cursor("loom_belief", second_seq, "2026-11-26T10:02:05Z")
                .expect("ack 2"),
            second_seq
        );
        assert_eq!(
            jobs.ack_consumer_cursor("loom_belief", first_seq, "2026-11-26T10:02:06Z")
                .expect("ack 1 no regress"),
            second_seq
        );
    }

    // Simulated daemon restart (re-open ops.fsqlite)
    {
        let store = OperationalStore::open(&db_path).expect("open 2");
        let jobs = DurableJobStore::new(&store);

        // Re-registering preserves persisted cursor
        assert!(
            !jobs
                .register_consumer("loom_belief", "2026-11-26T10:05:00Z")
                .expect("re-reg")
        );
        let cursor = jobs
            .consumer_cursor("loom_belief")
            .expect("cursor")
            .expect("exists");
        assert_eq!(cursor.last_acked_event_seq, 2);

        // loom_belief replays only event #3; cockpit_push (never acked) replays all 3
        let loom_remaining = jobs
            .replay_unacked_events("loom_belief")
            .expect("replay loom after crash");
        assert_eq!(loom_remaining.len(), 1);
        assert_eq!(loom_remaining[0].event_seq, 3);
        assert_eq!(loom_remaining[0].payload_json, "{\"idx\":3}");

        let cockpit_remaining = jobs
            .replay_unacked_events("cockpit_push")
            .expect("replay cockpit after crash");
        assert_eq!(cockpit_remaining.len(), 3);
    }
}

#[test]
fn gate_19_time_budget_pacing_watchdog_and_secret_scrubbing() {
    let now = 10_000_i64;

    // 1. TimeBudget: skip_after_spot_scoring_time, IntakeSkip (<120s), FastPath (<600s), Full (>=600s)
    let after_spot = TimeBudget::from_epochs(now, Some(now), now + 3_600);
    assert!(after_spot.skip_after_spot_scoring_time);
    assert_eq!(after_spot.mode, TimeBudgetMode::SkipAfterSpotScoring);
    assert!(after_spot.should_skip());

    let intake_skip = TimeBudget::from_epochs(now, None, now + MIN_INTAKE_SEC - 1);
    assert!(!intake_skip.skip_after_spot_scoring_time);
    assert_eq!(intake_skip.mode, TimeBudgetMode::IntakeSkip);
    assert!(intake_skip.should_skip());

    let fast_path =
        TimeBudget::from_epochs(now, Some(now + FAST_PATH_THRESHOLD_SEC - 1), now + 5_000);
    assert_eq!(fast_path.mode, TimeBudgetMode::FastPath);
    assert_eq!(fast_path.remaining_sec, FAST_PATH_THRESHOLD_SEC - 1);
    assert!(!fast_path.should_skip());

    let full = TimeBudget::from_epochs(now, Some(now + 3_600), now + 1_200);
    assert_eq!(full.mode, TimeBudgetMode::Full);
    assert_eq!(full.remaining_sec, 1_200);
    assert!(!full.should_skip());

    // QuestionRunContext spine assembly
    let ctx = QuestionRunContext {
        run_id: "run_001".into(),
        question_id: "q_eu_grid".into(),
        platform_id: "metaculus".into(),
        tournament_id: Some("futureeval_2026".into()),
        as_of_utc: "2026-11-26T12:00:00Z".into(),
        spot_scoring_utc: Some("2026-11-26T13:00:00Z".into()),
        close_utc: "2026-11-26T12:20:00Z".into(),
        time_budget: full,
        pacing_tier: BillingPacingTier::from_spend(2.0, 5.0),
        ab_slot: AbSlot::ControlA,
        prompt_cache_prefix_hash: "blake3:abcd1234".into(),
        donated_credit_pool: Some("futureeval".into()),
    };
    assert_eq!(ctx.pacing_tier, BillingPacingTier::Full);

    // 2. 4-tier BillingPacingTier
    assert_eq!(
        BillingPacingTier::from_spend(0.0, 5.0),
        BillingPacingTier::Full
    );
    assert_eq!(
        BillingPacingTier::from_spend(3.49, 5.0),
        BillingPacingTier::Full
    );
    assert_eq!(
        BillingPacingTier::from_spend(3.50, 5.0),
        BillingPacingTier::Economy
    );
    assert_eq!(
        BillingPacingTier::from_spend(4.49, 5.0),
        BillingPacingTier::Economy
    );
    assert_eq!(
        BillingPacingTier::from_spend(4.50, 5.0),
        BillingPacingTier::Minimal
    );
    assert_eq!(
        BillingPacingTier::from_spend(4.99, 5.0),
        BillingPacingTier::Minimal
    );
    assert_eq!(
        BillingPacingTier::from_spend(5.00, 5.0),
        BillingPacingTier::Stop
    );
    assert_eq!(
        BillingPacingTier::from_spend(6.00, 5.0),
        BillingPacingTier::Stop
    );
    assert_eq!(
        BillingPacingTier::from_spend(0.0, 0.0),
        BillingPacingTier::Stop
    );
    assert!(BillingPacingTier::Minimal.allows_dispatch());
    assert!(!BillingPacingTier::Stop.allows_dispatch());

    // 3. WatchdogAction (45 min alert, 75 min kill-and-reclaim)
    assert_eq!(
        WatchdogAction::evaluate(WATCHDOG_ALERT_SEC - 1),
        WatchdogAction::Healthy
    );
    assert_eq!(
        WatchdogAction::evaluate(WATCHDOG_ALERT_SEC),
        WatchdogAction::AlertAt45Min
    );
    assert_eq!(
        WatchdogAction::evaluate(WATCHDOG_KILL_RECLAIM_SEC - 1),
        WatchdogAction::AlertAt45Min
    );
    assert_eq!(
        WatchdogAction::evaluate(WATCHDOG_KILL_RECLAIM_SEC),
        WatchdogAction::KillAndReclaimAt75Min
    );

    // 4. Secret scrubbing across bearer, OAuth, API keys, and JSON/dead-letter payloads
    let raw_log = r#"HTTP 401 Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.secret.sig {"api_key": "AIzaSyD99999999999999", "oauth_token": "ya29.a0AfH6SMBxSecretToken", "openai": "sk-proj-1234567890abcdef", "github": "ghp_secret1234567890"}"#;
    let scrubbed = scrub_secrets(raw_log);
    assert!(!scrubbed.contains("eyJhbGciOiJIUzI1NiJ9"));
    assert!(!scrubbed.contains("AIzaSyD99999999999999"));
    assert!(!scrubbed.contains("ya29.a0AfH6SMBxSecretToken"));
    assert!(!scrubbed.contains("sk-proj-1234567890abcdef"));
    assert!(!scrubbed.contains("ghp_secret1234567890"));
    assert!(scrubbed.contains("[REDACTED]"));

    // Dead-letter persistence also scrubs secrets automatically
    let dir = TempDir::new().expect("tempdir");
    let store = OperationalStore::open(dir.path().join("db/ops.fsqlite")).expect("open");
    let jobs = DurableJobStore::new(&store);
    let (jid, _) = jobs
        .enqueue_idempotent(&JobSpec {
            kind: "deep_research".into(),
            lane: "epistemic_heavy".into(),
            priority: JobPriority::LiveShock,
            deadline_utc: "2026-11-26T12:00:00Z".into(),
            voi_log: 2.0,
            idempotency_seed: "seed:dlq_secret".into(),
            est_cost_usd: 0.1,
        })
        .expect("enqueue");
    jobs.record_failure(&jid, "provider error with Bearer tok_live_9876543210")
        .expect("fail");
    let stored_err: String = store
        .connection()
        .query_row(
            "SELECT last_error FROM fcast_jobs WHERE job_id = ?1",
            [&jid],
            |r| r.get(0),
        )
        .expect("read last_error");
    assert!(!stored_err.contains("tok_live_9876543210"));
    assert!(stored_err.contains("[REDACTED]"));
}
