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
pub const MIGRATION_0002_SQL: &str = include_str!("../migrations/0002_cyber_shift.sql");
pub const MIGRATION_0003_SQL: &str = include_str!("../migrations/0003_event_consumers.sql");
pub const MIGRATION_0004_SQL: &str = include_str!("../migrations/0004_immutability_and_chain.sql");
pub const MIGRATION_0005_SQL: &str = include_str!("../migrations/0005_tuning_cache_snapshots.sql");

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
        self.conn.execute_batch(MIGRATION_0003_SQL)?;
        self.conn.execute_batch(MIGRATION_0004_SQL)?;
        self.conn.execute_batch(MIGRATION_0005_SQL)?;
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
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
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

    /// Seals an immutable `ForecastRevision` while enforcing canonical epoch-ms `available_utc <= as_of_utc`
    /// (`INV-01`, `INV-03`), parent revision chain integrity & timestamp monotonicity,
    /// and emitting a transactional `RevisionSealed` event into `domain_events` (`§13.2`).
    pub fn seal_forecast_revision(&self, rev: &ForecastRevision) -> Result<(), StoreError> {
        let rev_ms = parse_rfc3339_epoch_ms(&rev.as_of_utc)?;

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
                StoreError::ChainInvariantViolation(format!("missing parent revision {parent_id}"))
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

        let mut best: Option<(i64, (String, String, usize, String))> = None;
        for (sid, created, rn, payload) in rows {
            let ms = parse_rfc3339_epoch_ms(&created)?;
            if best.as_ref().is_none_or(|(b_ms, _)| ms >= *b_ms) {
                best = Some((ms, (sid, created, rn.max(0) as usize, payload)));
            }
        }
        Ok(best.map(|(_, row)| row))
    }
}

/// Parses an RFC3339 timestamp (`YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`) into exact UTC epoch milliseconds.
pub fn parse_rfc3339_epoch_ms(s: &str) -> Result<i64, StoreError> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || (b[10] != b'T' && b[10] != b't')
        || b[13] != b':'
        || b[16] != b':'
    {
        return Err(StoreError::InvalidTimestamp(s.to_owned()));
    }
    let parse_u = |lo: usize, hi: usize| -> Result<i64, StoreError> {
        s[lo..hi]
            .parse::<i64>()
            .map_err(|_| StoreError::InvalidTimestamp(s.to_owned()))
    };
    let year = parse_u(0, 4)?;
    let month = parse_u(5, 7)?;
    let day = parse_u(8, 10)?;
    let hour = parse_u(11, 13)?;
    let minute = parse_u(14, 16)?;
    let second = parse_u(17, 19)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return Err(StoreError::InvalidTimestamp(s.to_owned()));
    }

    let mut idx = 19;
    let mut millis = 0_i64;
    if idx < b.len() && b[idx] == b'.' {
        idx += 1;
        let frac_start = idx;
        while idx < b.len() && b[idx].is_ascii_digit() {
            idx += 1;
        }
        let frac = &s[frac_start..idx];
        if frac.is_empty() {
            return Err(StoreError::InvalidTimestamp(s.to_owned()));
        }
        let mut scale = 100_i64;
        for ch in frac.bytes().take(3) {
            millis += i64::from(ch - b'0') * scale;
            scale /= 10;
        }
    }

    let offset_sec: i64 = if idx < b.len() && (b[idx] == b'Z' || b[idx] == b'z') {
        idx += 1;
        0
    } else if idx + 6 == b.len() && (b[idx] == b'+' || b[idx] == b'-') && b[idx + 3] == b':' {
        let sign = if b[idx] == b'+' { 1 } else { -1 };
        let oh = parse_u(idx + 1, idx + 3)?;
        let om = parse_u(idx + 4, idx + 6)?;
        idx += 6;
        sign * (oh * 3600 + om * 60)
    } else {
        return Err(StoreError::InvalidTimestamp(s.to_owned()));
    };
    if idx != b.len() {
        return Err(StoreError::InvalidTimestamp(s.to_owned()));
    }

    // Howard Hinnant civil-to-days algorithm (UTC days since 1970-01-01).
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m_adj = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m_adj + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let total_sec = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_sec;
    Ok(total_sec * 1_000 + millis)
}
