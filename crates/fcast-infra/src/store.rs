//! `Chronicle` `OperationalStore` over `fsqlite::blocking` WAL/MVCC (`§7`, `§13.2`, `§18`).

use crate::vault::VaultedArtifact;
use fcast_core::forecast::{
    ForecastRevision, OutcomeDistribution, ProbClamped, ResolutionStatusAtCutoff,
};
use fsqlite::blocking::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MIGRATION_0001_SQL: &str = include_str!("../migrations/0001_init.sql");
pub const MIGRATION_0002_SQL: &str = include_str!("../migrations/0002_cyber_shift.sql");
pub const MIGRATION_0003_SQL: &str = include_str!("../migrations/0003_event_consumers.sql");
pub const MIGRATION_0004_SQL: &str = include_str!("../migrations/0004_immutability_and_chain.sql");
pub const MIGRATION_0005_SQL: &str = include_str!("../migrations/0005_tuning_cache_snapshots.sql");
pub const MIGRATION_0006_SQL: &str = include_str!("../migrations/0006_indexes.sql");

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
    #[error(
        "TEMPORAL_FENCE_VIOLATION: handle {handle} available_utc={available_utc} > as_of_utc={as_of_utc}"
    )]
    TemporalFenceViolation {
        handle: String,
        available_utc: String,
        as_of_utc: String,
    },
    #[error("invalid RFC3339 timestamp: {0}")]
    InvalidTimestamp(String),
    #[error("revision chain invariant violated: {0}")]
    ChainInvariantViolation(String),
    #[error("missing EvidenceVault index handle: {0}")]
    MissingEvidenceHandle(String),
    #[error("integrity check failed: {0}")]
    IntegrityFailed(String),
    #[error(
        "daily budget circuit breaker tripped: spent={spent_usd:.2} + est={est_usd:.2} > cap={cap_usd:.2}"
    )]
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
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA wal_autocheckpoint = 1000;",
        )?;

        let store = Self { conn, db_path };
        store.apply_initial_migration()?;
        Ok(store)
    }

    fn apply_initial_migration(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(MIGRATION_0001_SQL)?;
        self.conn.execute_batch(MIGRATION_0003_SQL)?;
        self.conn.execute_batch(MIGRATION_0004_SQL)?;
        self.conn.execute_batch(MIGRATION_0005_SQL)?;
        self.conn.execute_batch(MIGRATION_0006_SQL)?;
        let sha = format!("{:x}", Sha256::digest(MIGRATION_0001_SQL.as_bytes()));
        self.conn.execute(
            "INSERT OR IGNORE INTO _fcast_migrations (version, name, sha256_hex, applied_utc)
             VALUES (?1, ?2, ?3, ?4)",
            params![1_i64, "0001_init", sha, "2026-11-25T00:00:00Z"],
        )?;
        let sha4 = format!("{:x}", Sha256::digest(MIGRATION_0004_SQL.as_bytes()));
        self.conn.execute(
            "INSERT OR IGNORE INTO _fcast_migrations (version, name, sha256_hex, applied_utc)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                4_i64,
                "0004_immutability_and_chain",
                sha4,
                "2026-11-26T00:00:00Z"
            ],
        )?;
        let sha5 = format!("{:x}", Sha256::digest(MIGRATION_0005_SQL.as_bytes()));
        self.conn.execute(
            "INSERT OR IGNORE INTO _fcast_migrations (version, name, sha256_hex, applied_utc)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                5_i64,
                "0005_tuning_cache_snapshots",
                sha5,
                "2026-11-26T00:00:00Z"
            ],
        )?;
        let sha6 = format!("{:x}", Sha256::digest(MIGRATION_0006_SQL.as_bytes()));
        self.conn.execute(
            "INSERT OR IGNORE INTO _fcast_migrations (version, name, sha256_hex, applied_utc)
             VALUES (?1, ?2, ?3, ?4)",
            params![6_i64, "0006_indexes", sha6, "2026-11-27T00:00:00Z"],
        )?;
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Verifies `PRAGMA integrity_check == "ok"`, checkpoints the WAL (`TRUNCATE`) so all
    /// committed `ForecastRevision` rows are flushed into the main DB file, and writes a
    /// `Zstd-9` backup + `.sha256` sidecar (`GATE-02`).
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
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(StoreError::IntegrityFailed(format!(
                "WAL checkpoint busy ({busy}); refusing inconsistent backup"
            )));
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

    /// Seals an immutable `ForecastRevision` atomically inside `BEGIN IMMEDIATE` while enforcing
    /// canonical epoch-ms `available_utc <= as_of_utc` (`INV-01`, `INV-03`), parent revision chain
    /// integrity & timestamp monotonicity, and emitting a transactional `RevisionSealed` event (`§13.2`).
    pub fn seal_forecast_revision(&self, rev: &ForecastRevision) -> Result<(), StoreError> {
        // Re-verify canonical BLAKE3 revision_id, lifecycle, and distribution validity so
        // in-memory mutations or bypassed seal paths can never persist a forged revision.
        rev.verify_integrity()?;
        let rev_ms = parse_rfc3339_epoch_ms(&rev.as_of_utc)?;
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
            if let Some(parent_id) = &rev.parent_revision_id {
                let parent_row: Option<(String, i64, String)> = self
                    .conn
                    .query_row(
                        "SELECT forecast_id, seq_no, as_of_utc FROM forecast_revisions WHERE revision_id = ?1",
                        params![parent_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let (p_fc_id, p_seq, p_as_of) = parent_row.ok_or_else(|| {
                    StoreError::ChainInvariantViolation(format!(
                        "missing parent revision {parent_id}"
                    ))
                })?;
                if p_fc_id != rev.forecast_id {
                    return Err(StoreError::ChainInvariantViolation(format!(
                        "parent {parent_id} belongs to forecast {p_fc_id}, not {}",
                        rev.forecast_id
                    )));
                }
                if p_seq + 1 != i64::from(rev.seq_no) {
                    return Err(StoreError::ChainInvariantViolation(format!(
                        "non-contiguous seq_no: parent={p_seq}, child={}",
                        rev.seq_no
                    )));
                }
                let p_ms = parse_rfc3339_epoch_ms(&p_as_of)?;
                if rev_ms < p_ms {
                    return Err(StoreError::ChainInvariantViolation(format!(
                        "non-monotonic as_of_utc: child={} < parent={p_as_of}",
                        rev.as_of_utc
                    )));
                }
            }

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
                let avail_ms = parse_rfc3339_epoch_ms(&avail)?;
                if avail_ms > rev_ms {
                    return Err(StoreError::TemporalFenceViolation {
                        handle: handle.as_str().to_owned(),
                        available_utc: avail,
                        as_of_utc: rev.as_of_utc.clone(),
                    });
                }
            }

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
                    "UPDATE forecast_streams SET origin_revision_id = ?1 WHERE forecast_id = ?2 AND origin_revision_id IS NULL",
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
    /// Compares timestamps by exact UTC epoch milliseconds so fractional seconds and offsets never skew ordering.
    pub fn project_at_horizon(
        &self,
        forecast_id: &str,
        eval_as_of_utc: &str,
    ) -> Result<HorizonProjection, StoreError> {
        let eval_ms = parse_rfc3339_epoch_ms(eval_as_of_utc)?;
        type RowTuple = (String, i64, String, f64, Option<f64>, String, String);
        let mut stmt = self.conn.prepare(
            "SELECT revision_id, seq_no, as_of_utc, p_indep, p_market, probability_distribution, resolution_status
             FROM forecast_revisions
             WHERE forecast_id = ?1",
        )?;
        let rows: Vec<RowTuple> = stmt
            .query_map(params![forecast_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut best: Option<(i64, i64, RowTuple)> = None;
        for row in rows {
            let row_ms = parse_rfc3339_epoch_ms(&row.2)?;
            if row_ms <= eval_ms {
                let key = (row_ms, row.1);
                if best
                    .as_ref()
                    .is_none_or(|(b_ms, b_seq, _)| key > (*b_ms, *b_seq))
                {
                    best = Some((row_ms, row.1, row));
                }
            }
        }

        let Some((
            _,
            _,
            (revision_id, seq_no, as_of_utc, p_indep_f, p_market_f, dist_str, status_s),
        )) = best
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

    /// Reads `(spent_usd, cap_usd)` from `daemon_budget_ledger` for `utc_day` (defaults to `(0.0, 5.0)`).
    pub fn get_budget_status(&self, utc_day: &str) -> Result<(f64, f64), StoreError> {
        let row: Option<(f64, f64)> = self
            .conn
            .query_row(
                "SELECT spent_usd, cap_usd FROM daemon_budget_ledger WHERE utc_day = ?1",
                params![utc_day],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.unwrap_or((0.0, 5.0)))
    }

    /// Atomically records realized metered LLM/API spend in `daemon_budget_ledger` (`§13`).
    pub fn record_budget_spend(
        &self,
        utc_day: &str,
        delta_usd: f64,
        default_cap_usd: f64,
    ) -> Result<(f64, f64), StoreError> {
        if !delta_usd.is_finite()
            || delta_usd < 0.0
            || !default_cap_usd.is_finite()
            || default_cap_usd <= 0.0
        {
            return Err(StoreError::IntegrityFailed(format!(
                "invalid budget spend parameters: delta={delta_usd}, cap={default_cap_usd}"
            )));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let res = (|| -> Result<(f64, f64), StoreError> {
            self.conn.execute(
                "INSERT OR IGNORE INTO daemon_budget_ledger (utc_day, spent_usd, cap_usd)
                 VALUES (?1, 0.0, ?2)",
                params![utc_day, default_cap_usd],
            )?;
            self.conn.execute(
                "UPDATE daemon_budget_ledger SET spent_usd = spent_usd + ?1 WHERE utc_day = ?2",
                params![delta_usd, utc_day],
            )?;
            self.get_budget_status(utc_day)
        })();
        match res {
            Ok(pair) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(pair)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Checks daily spend circuit breaker (`$5.00/day` default cap) before metered job dispatch.
    /// Fail-closed on non-finite or negative `est_usd` estimates.
    pub fn check_budget_available(&self, utc_day: &str, est_usd: f64) -> Result<(), StoreError> {
        if !est_usd.is_finite() || est_usd < 0.0 {
            return Err(StoreError::IntegrityFailed(format!(
                "invalid estimated job cost: {est_usd}"
            )));
        }
        let (spent_usd, cap_usd) = self.get_budget_status(utc_day)?;
        if !spent_usd.is_finite() || !cap_usd.is_finite() || spent_usd + est_usd > cap_usd {
            return Err(StoreError::BudgetCapExceeded {
                spent_usd,
                est_usd,
                cap_usd,
            });
        }
        Ok(())
    }

    /// Applies forward-only `0002_cyber_shift.sql` strictly AFTER a verified online `Zstd-9` backup (`INV-12`, `GATE-02`).
    pub fn apply_migration_0002_with_backup(
        &self,
        backup_path: impl AsRef<Path>,
    ) -> Result<BackupReceipt, StoreError> {
        let already: Option<i64> = self
            .conn
            .query_row(
                "SELECT version FROM _fcast_migrations WHERE version = 2",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let receipt = self.create_verified_zstd_backup(backup_path)?;
        if already.is_none() {
            self.conn.execute_batch(MIGRATION_0002_SQL)?;
            let sha = format!("{:x}", Sha256::digest(MIGRATION_0002_SQL.as_bytes()));
            self.conn.execute(
                "INSERT INTO _fcast_migrations (version, name, sha256_hex, applied_utc)
                 VALUES (?1, ?2, ?3, ?4)",
                params![2_i64, "0002_cyber_shift", sha, "2026-11-25T00:00:00Z"],
            )?;
        }
        Ok(receipt)
    }

    /// Applies forward-only `0003_event_consumers.sql` idempotently and records version `3`
    /// in `_fcast_migrations` (`§13.2`, `Plan v2.1`).
    pub fn apply_migration_0003(&self) -> Result<(), StoreError> {
        let already: Option<i64> = self
            .conn
            .query_row(
                "SELECT version FROM _fcast_migrations WHERE version = 3",
                [],
                |r| r.get(0),
            )
            .optional()?;
        self.conn.execute_batch(MIGRATION_0003_SQL)?;
        if already.is_none() {
            let sha = format!("{:x}", Sha256::digest(MIGRATION_0003_SQL.as_bytes()));
            self.conn.execute(
                "INSERT INTO _fcast_migrations (version, name, sha256_hex, applied_utc)
                 VALUES (?1, ?2, ?3, ?4)",
                params![3_i64, "0003_event_consumers", sha, "2026-11-26T00:00:00Z"],
            )?;
        }
        Ok(())
    }

    /// Lossless `V1 -> V2` read upcaster (`§18`): reads immutable historical `V1` revision payload
    /// alongside its upcasted `cyber_prior_shift_nats` (`0.0` default) without mutating `forecast_revisions`.
    pub fn read_revision_upcast_v2(
        &self,
        revision_id: &str,
    ) -> Result<Option<(i64, f64, f64)>, StoreError> {
        let row: Option<(i64, f64, Option<f64>)> = self
            .conn
            .query_row(
                "SELECT r.payload_schema_ver, r.p_indep, s.cyber_prior_shift_nats
                 FROM forecast_revisions r
                 JOIN forecast_streams s ON s.forecast_id = r.forecast_id
                 WHERE r.revision_id = ?1",
                params![revision_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.map(|(ver, p_indep, shift)| (ver, p_indep, shift.unwrap_or(0.0))))
    }

    /// Idempotently inserts a question record (`INSERT OR IGNORE`).
    pub fn upsert_question_if_absent(
        &self,
        question_id: &str,
        venue: &str,
        title: &str,
        contract_sha256: &str,
        horizon_geometry: &str,
        created_utc: &str,
    ) -> Result<(), StoreError> {
        parse_rfc3339_epoch_ms(created_utc)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO questions (question_id, venue, title, contract_sha256, horizon_geometry, created_utc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                question_id,
                venue,
                title,
                contract_sha256,
                horizon_geometry,
                created_utc
            ],
        )?;
        Ok(())
    }

    /// Idempotently registers a vaulted blob in `evidence_vault_index` with validated RFC-3339 `available_utc`.
    pub fn index_vault_artifact(
        &self,
        art: &VaultedArtifact,
        available_utc: &str,
    ) -> Result<(), StoreError> {
        parse_rfc3339_epoch_ms(available_utc)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO evidence_vault_index (ev_handle, sha256_hex, span_id, available_utc, byte_len, zstd_compressed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                art.ev_handle.as_str(),
                &art.sha256_hex,
                &art.span_id,
                available_utc,
                art.byte_len as i64,
                i64::from(art.zstd_compressed),
            ],
        )?;
        Ok(())
    }

    /// Idempotently ensures a `forecast_streams` row exists for `(forecast_id, question_id)`.
    pub fn ensure_forecast_stream(
        &self,
        forecast_id: &str,
        question_id: &str,
        stream_kind: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO forecast_streams (forecast_id, question_id, stream_kind, origin_revision_id)
             VALUES (?1, ?2, ?3, NULL)",
            params![forecast_id, question_id, stream_kind],
        )?;
        Ok(())
    }

    /// Persists an append-only immutable `TuningCacheSnapshot` (`0005_tuning_cache_snapshots.sql`).
    /// `RepRouter(t)` reads only pre-committed snapshots `(t-1)`, decoupling `FR4` from live `Crucible` (`DP8`).
    pub fn save_tuning_snapshot(
        &self,
        snapshot_id: &str,
        created_utc: &str,
        resolved_n: usize,
        payload_json: &str,
    ) -> Result<(), StoreError> {
        parse_rfc3339_epoch_ms(created_utc)?;
        self.conn.execute(
            "INSERT INTO rep_router_tuning_snapshots (snapshot_id, created_utc, resolved_n, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![snapshot_id, created_utc, resolved_n as i64, payload_json],
        )?;
        Ok(())
    }

    /// Idempotent variant of [`Self::save_tuning_snapshot`] (`INSERT OR IGNORE`) that preserves
    /// immutability while propagating real database errors instead of swallowing them.
    pub fn ensure_tuning_snapshot_idempotent(
        &self,
        snapshot_id: &str,
        created_utc: &str,
        resolved_n: usize,
        payload_json: &str,
    ) -> Result<bool, StoreError> {
        parse_rfc3339_epoch_ms(created_utc)?;
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO rep_router_tuning_snapshots (snapshot_id, created_utc, resolved_n, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![snapshot_id, created_utc, resolved_n as i64, payload_json],
        )?;
        Ok(inserted > 0)
    }

    /// Loads the latest immutable `TuningCacheSnapshot` as `(snapshot_id, created_utc, resolved_n, payload_json)`.
    pub fn load_latest_tuning_snapshot(
        &self,
    ) -> Result<Option<(String, String, usize, String)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT snapshot_id, created_utc, resolved_n, payload_json
             FROM rep_router_tuning_snapshots",
        )?;
        let rows: Vec<(String, String, i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;

        type SnapshotRow = (String, String, usize, String);
        let mut best: Option<(i64, String, SnapshotRow)> = None;
        for (sid, created, rn, payload) in rows {
            let ms = parse_rfc3339_epoch_ms(&created)?;
            if best
                .as_ref()
                .is_none_or(|(b_ms, b_sid, _)| (ms, sid.as_str()) > (*b_ms, b_sid.as_str()))
            {
                best = Some((ms, sid.clone(), (sid, created, rn.max(0) as usize, payload)));
            }
        }
        Ok(best.map(|(_, _, row)| row))
    }
}

/// Parses an RFC3339 timestamp (`YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`) into exact UTC epoch milliseconds.
pub fn parse_rfc3339_epoch_ms(s: &str) -> Result<i64, StoreError> {
    fcast_core::time::parse_rfc3339_epoch_ms(s)
        .map_err(|_| StoreError::InvalidTimestamp(s.to_owned()))
}
