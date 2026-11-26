//! `GATE-02`, `GATE-04`, `GATE-05`: WAL backup, ForecastRevision immutability, causal fence, and HorizonForecastView.

use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::forecast::{
    OutcomeDistribution, ProbClamped, ResolutionStatusAtCutoff, UnsealedForecastRevision,
};
use fcast_infra::store::{HorizonProjection, OperationalStore, StoreError};
use fsqlite::blocking::params;
use tempfile::TempDir;

#[test]
fn gate_02_04_05_store_immutability_fence_and_horizon_projection() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("db/ops.fsqlite");
    let store = OperationalStore::open(&db_path).expect("open store");
    let conn = store.connection();

    conn.execute(
        "INSERT INTO questions VALUES ('q1', 'metaculus', 'Baltic Test', 'sha_c1', 'point_in_time', '2026-11-01T00:00:00Z')",
        [],
    )
    .expect("insert q");
    conn.execute(
        "INSERT INTO forecast_streams VALUES ('fc1', 'q1', 'live_tournament', NULL)",
        [],
    )
    .expect("insert stream");
    for (h, ts) in [
        ("[ev:0101#ov]", "2026-11-01T09:00:00Z"),
        ("[ev:0202#rat0]", "2026-11-01T10:00:00Z"),
        ("[ev:0303#future]", "2026-11-20T00:00:00Z"),
    ] {
        conn.execute(
            "INSERT INTO evidence_vault_index VALUES (?1, 'deadbeef', 's', ?2, 10, 0)",
            params![h, ts],
        )
        .expect("insert ev");
    }

    let r0 = UnsealedForecastRevision {
        forecast_id: "fc1".into(),
        seq_no: 0,
        parent_revision_id: None,
        question_id: "q1".into(),
        as_of_utc: "2026-11-01T10:00:00Z".into(),
        p_indep: ProbClamped::new(0.20).expect("p"),
        p_market: Some(ProbClamped::new(0.18).expect("p")),
        probability_distribution: OutcomeDistribution::Binary {
            p: ProbClamped::new(0.19).expect("p"),
            n_eff: 8.0,
        },
        evidence_handles: vec![VerifiedEvidenceRef::parse("[ev:0101#ov]").expect("ev")],
        model_version: "gemini-4-argon@2026-05-20".into(),
        calibration_version: "cal-v1".into(),
        rationale_handle: VerifiedEvidenceRef::parse("[ev:0202#rat0]").expect("rat"),
        resolution_status: ResolutionStatusAtCutoff::Unresolved,
    }
    .seal()
    .expect("seal r0");

    store.seal_forecast_revision(&r0).expect("persist r0");

    // Causal fence MUST reject future evidence handle
    let bad_r1 = UnsealedForecastRevision {
        forecast_id: "fc1".into(),
        seq_no: 1,
        parent_revision_id: Some(r0.revision_id.clone()),
        question_id: "q1".into(),
        as_of_utc: "2026-11-10T00:00:00Z".into(),
        p_indep: ProbClamped::new(0.55).expect("p"),
        p_market: None,
        probability_distribution: OutcomeDistribution::Binary {
            p: ProbClamped::new(0.55).expect("p"),
            n_eff: 10.0,
        },
        evidence_handles: vec![VerifiedEvidenceRef::parse("[ev:0303#future]").expect("ev")],
        model_version: "gemini-4-argon@2026-05-20".into(),
        calibration_version: "cal-v1".into(),
        rationale_handle: VerifiedEvidenceRef::parse("[ev:0202#rat0]").expect("rat"),
        resolution_status: ResolutionStatusAtCutoff::Unresolved,
    }
    .seal()
    .expect("seal bad_r1");
    assert!(matches!(
        store.seal_forecast_revision(&bad_r1),
        Err(StoreError::TemporalFenceViolation { .. })
    ));

    // Immutability triggers MUST abort UPDATE and DELETE
    assert!(
        conn.execute("UPDATE forecast_revisions SET p_indep = 0.99", [])
            .is_err()
    );
    assert!(conn.execute("DELETE FROM forecast_revisions", []).is_err());

    // Horizon projection returns NotYetIssued before origin and StandingRevision after
    assert_eq!(
        store
            .project_at_horizon("fc1", "2026-10-31T23:59:59Z")
            .expect("proj pre"),
        HorizonProjection::NotYetIssued
    );
    assert!(matches!(
        store
            .project_at_horizon("fc1", "2026-11-05T00:00:00Z")
            .expect("proj post"),
        HorizonProjection::StandingRevision { seq_no: 0, .. }
    ));

    // Verified online Zstd-9 backup
    let bak = store
        .create_verified_zstd_backup(dir.path().join("backups/ops-pre-v2.fsqlite.zst"))
        .expect("backup");
    assert!(bak.backup_path.exists());
    assert_eq!(bak.sha256_hex.len(), 64);
}
