//! `Chronicle` `OperationalStore` over `fsqlite::blocking` WAL/MVCC (`§7`, `§13.2`, `§18`).

use fcast_core::forecast::{
    ForecastRevision, OutcomeDistribution, ProbClamped, ResolutionStatusAtCutoff,
};
use fsqlite::blocking::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MIGRATION_0001_SQL: &str = include_str!("../migrations/0001_init.sql");

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("fsqlite error: {0}")]
    Db(#[from] fsqlite::blocking::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("core forecast error: {0}")]
    Core(#[from] fcast_core::forecast::ForecastError),
    #[error("blinding error: {0}")]
    Blinding(#[from] fcast_core::blinding::BlindingError),
    #[error("TEMPORAL_FENCE_VIOLATION: handle {handle} available_utc={available_utc} > as_of_utc={as_of_utc}")]
    TemporalFenceViolation {
        handle: String,
        available_utc: String,
        as_of_utc: String,
    },
    #[error("missing EvidenceVault index handle: {0}")]
    MissingEvidenceHandle(String),
    #[error("integrity check failed: {0}")]
    IntegrityFailed(String),
    #[error("daily budget circuit breaker tripped: spent={spent_usd:.2} + est={est_usd:.2} > cap={cap_usd:.2}")]
    BudgetCapExceeded {
        spent_usd: f64,
        est_usd: f64,
        cap_usd: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HorizonProjection {
    NotYetIssued,
    StandingRevision {
        revision_id: String,
        seq_no: u32,
        as_of_utc: String,
        p_indep: ProbClamped,
        p_market: Option<ProbClamped>,
        distribution: OutcomeDistribution,
        resolution_status: ResolutionStatusAtCutoff,
    },
}

#[derive(Debug, Clone)]
pub struct BackupReceipt {
    pub backup_path: PathBuf,
    pub sha256_hex: String,
}

pub struct OperationalStore {
    conn: Connection,
    db_path: PathBuf,
}

impl std::fmt::Debug for OperationalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationalStore")
            .field("db_path", &self.db_path)
            .finish()
    }
}

impl OperationalStore {
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA wal_autocheckpoint = 1000;",
        )?;

        let store = Self { conn, db_path };
        store.apply_initial_migration()?;
        Ok(store)
    }

    fn apply_initial_migration(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(MIGRATION_0001_SQL)?;
        let sha = format!("{:x}", Sha256::digest(MIGRATION_0001_SQL.as_bytes()));
        self.conn.execute(
            "INSERT OR IGNORE INTO _fcast_migrations (version, name, sha256_hex, applied_utc)
             VALUES (?1, ?2, ?3, ?4)",
            params![1_i64, "0001_init", sha, "2026-11-25T00:00:00Z"],
        )?;
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Verifies `PRAGMA integrity_check == "ok"` and writes a `Zstd-9` backup + `.sha256` sidecar (`GATE-02`).
    pub fn create_verified_zstd_backup(
        &self,
        backup_path: impl AsRef<Path>,
    ) -> Result<BackupReceipt, StoreError> {
        let status: String = self
            .conn
            .query_row("PRAGMA integrity_check;", [], |row| row.get(0))?;
        if !status.eq_ignore_ascii_case("ok") {
            return Err(StoreError::IntegrityFailed(status));
        }
        let raw = fs::read(&self.db_path)?;
        let compressed = zstd::bulk::compress(&raw, 9)?;
        let backup_path = backup_path.as_ref().to_path_buf();
        if let Some(parent) = backup_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&backup_path, &compressed)?;
        let sha256_hex = format!("{:x}", Sha256::digest(&compressed));
        let sidecar = backup_path.with_extension("zst.sha256");
        fs::write(sidecar, format!("{sha256_hex}\n"))?;
        Ok(BackupReceipt {
            backup_path,
            sha256_hex,
        })
    }

    /// Seals an immutable `ForecastRevision` while enforcing `available_utc <= as_of_utc` (`INV-01`, `INV-03`)
    /// and emitting a transactional `RevisionSealed` event into `domain_events` (`§13.2`).
    pub fn seal_forecast_revision(&self, rev: &ForecastRevision) -> Result<(), StoreError> {
        for handle in rev
            .evidence_handles
            .iter()
            .chain(std::iter::once(&rev.rationale_handle))
        {
            let avail: Option<String> = self
                .conn
                .query_row(
                    "SELECT available_utc FROM evidence_vault_index WHERE ev_handle = ?1",
                    params![handle.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let avail = avail
                .ok_or_else(|| StoreError::MissingEvidenceHandle(handle.as_str().to_owned()))?;
            if avail.as_str() > rev.as_of_utc.as_str() {
                return Err(StoreError::TemporalFenceViolation {
                    handle: handle.as_str().to_owned(),
                    available_utc: avail,
                    as_of_utc: rev.as_of_utc.clone(),
                });
            }
        }

        let dist_json = serde_json::to_string(&rev.probability_distribution)?;
        let ev_json = serde_json::to_string(&rev.evidence_handles)?;
        let status_str = match rev.resolution_status {
            ResolutionStatusAtCutoff::Unresolved => "unresolved",
            ResolutionStatusAtCutoff::Resolved => "resolved",
            ResolutionStatusAtCutoff::Invalidated => "invalidated",
        };
        let p_market_val: Option<f64> = rev.p_market.map(ProbClamped::get);

        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let res = (|| -> Result<(), StoreError> {
            self.conn.execute(
                "INSERT INTO forecast_revisions (
                    revision_id, forecast_id, seq_no, parent_revision_id, question_id,
                    as_of_utc, p_indep, p_market, probability_distribution, evidence_handles,
                    model_version, calibration_version, rationale_handle, resolution_status, payload_schema_ver
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,1)",
                params![
                    &rev.revision_id,
                    &rev.forecast_id,
                    i64::from(rev.seq_no),
                    &rev.parent_revision_id,
                    &rev.question_id,
                    &rev.as_of_utc,
                    rev.p_indep.get(),
                    p_market_val,
                    &dist_json,
                    &ev_json,
                    &rev.model_version,
                    &rev.calibration_version,
                    rev.rationale_handle.as_str(),
                    status_str,
                ],
            )?;
            if rev.seq_no == 0 {
                self.conn.execute(
                    "UPDATE forecast_streams SET origin_revision_id = ?1 WHERE forecast_id = ?2",
                    params![&rev.revision_id, &rev.forecast_id],
                )?;
            }
            let event_payload = serde_json::json!({
                "revision_id": &rev.revision_id,
                "seq_no": rev.seq_no,
                "p_indep": rev.p_indep.get(),
            })
            .to_string();
            let idem = format!("rev_sealed:{}", rev.revision_id);
            self.conn.execute(
                "INSERT OR IGNORE INTO domain_events (event_kind, question_id, forecast_id, as_of_utc, idempotency_key, payload_json)
                 VALUES ('RevisionSealed', ?1, ?2, ?3, ?4, ?5)",
                params![
                    &rev.question_id,
                    &rev.forecast_id,
                    &rev.as_of_utc,
                    &idem,
                    &event_payload,
                ],
            )?;
            Ok(())
        })();

        match res {
            Ok(()) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Stage-3 pure horizon projection (`HorizonForecastView`, `INV-02`, `GATE-05`).
    pub fn project_at_horizon(
        &self,
        forecast_id: &str,
        eval_as_of_utc: &str,
    ) -> Result<HorizonProjection, StoreError> {
        type RowTuple = (String, i64, String, f64, Option<f64>, String, String);
        let row: Option<RowTuple> = self
            .conn
            .query_row(
                "SELECT revision_id, seq_no, as_of_utc, p_indep, p_market, probability_distribution, resolution_status
                 FROM forecast_revisions
                 WHERE forecast_id = ?1 AND as_of_utc <= ?2
                 ORDER BY as_of_utc DESC, seq_no DESC LIMIT 1",
                params![forecast_id, eval_as_of_utc],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;

        let Some((revision_id, seq_no, as_of_utc, p_indep_f, p_market_f, dist_str, status_s)) = row
        else {
            return Ok(HorizonProjection::NotYetIssued);
        };
        let p_indep = ProbClamped::new(p_indep_f)?;
        let p_market = match p_market_f {
            Some(f) => Some(ProbClamped::new(f)?),
            None => None,
        };
        let distribution: OutcomeDistribution = serde_json::from_str(&dist_str)?;
        let resolution_status = match status_s.as_str() {
            "resolved" => ResolutionStatusAtCutoff::Resolved,
            "invalidated" => ResolutionStatusAtCutoff::Invalidated,
            _ => ResolutionStatusAtCutoff::Unresolved,
        };
        Ok(HorizonProjection::StandingRevision {
            revision_id,
            seq_no: seq_no as u32,
            as_of_utc,
            p_indep,
            p_market,
            distribution,
            resolution_status,
        })
    }

    /// Checks daily spend circuit breaker (`$5.00/day` default cap) before metered job dispatch.
    pub fn check_budget_available(&self, utc_day: &str, est_usd: f64) -> Result<(), StoreError> {
        let row: Option<(f64, f64)> = self
            .conn
            .query_row(
                "SELECT spent_usd, cap_usd FROM daemon_budget_ledger WHERE utc_day = ?1",
                params![utc_day],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (spent_usd, cap_usd) = row.unwrap_or((0.0, 5.0));
        if spent_usd + est_usd > cap_usd {
            return Err(StoreError::BudgetCapExceeded {
                spent_usd,
                est_usd,
                cap_usd,
            });
        }
        Ok(())
    }
}
