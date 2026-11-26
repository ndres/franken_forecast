//! `Metronome` `DurableJobStore` & transactional `domain_events` outbox consumer (`§13`, `§13.2`).

use crate::store::{OperationalStore, StoreError};
use fsqlite::blocking::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub const MAX_JOB_ATTEMPTS: i64 = 3;

/// Five-tier priority preemption schedule (`§13.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(i64)]
pub enum JobPriority {
    Maintenance = 0,
    ScheduledSync = 1,
    LiveShock = 2,
    RedTeamOrManual = 3,
    TournamentFreeze = 4,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSpec {
    pub kind: String,
    pub lane: String,
    pub priority: JobPriority,
    pub deadline_utc: String,
    pub voi_log: f64,
    pub idempotency_seed: String,
    pub est_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedJob {
    pub job_id: String,
    pub kind: String,
    pub lane: String,
    pub priority_tier: i64,
    pub deadline_utc: String,
    pub voi_log: f64,
    pub attempts: i64,
    pub est_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainEventRecord {
    pub event_seq: i64,
    pub event_kind: String,
    pub question_id: String,
    pub forecast_id: Option<String>,
    pub as_of_utc: String,
    pub idempotency_key: String,
    pub payload_json: String,
}

#[derive(Debug)]
pub struct DurableJobStore<'a> {
    store: &'a OperationalStore,
}

impl<'a> DurableJobStore<'a> {
    #[must_use]
    pub const fn new(store: &'a OperationalStore) -> Self {
        Self { store }
    }

    /// Enqueues a job idempotently using `BLAKE3(idempotency_seed)`; returns `true` if newly inserted.
    pub fn enqueue_idempotent(&self, spec: &JobSpec) -> Result<(String, bool), StoreError> {
        let idem_hash = blake3::hash(spec.idempotency_seed.as_bytes())
            .to_hex()
            .to_string();
        let job_id = format!("job_{}", &idem_hash[0..16]);
        let changed = self.store.connection().execute(
            "INSERT OR IGNORE INTO fcast_jobs
             (job_id, kind, lane, priority_tier, deadline_utc, voi_log, idempotency_key, state, lease_expires_epoch, attempts, est_cost_usd, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'QUEUED', NULL, 0, ?8, NULL)",
            params![
                &job_id,
                &spec.kind,
                &spec.lane,
                spec.priority as i64,
                &spec.deadline_utc,
                spec.voi_log,
                &idem_hash,
                spec.est_cost_usd,
            ],
        )?;
        Ok((job_id, changed > 0))
    }

    /// Dequeues the highest-priority `QUEUED` job (`priority_tier DESC, deadline_utc ASC, voi_log DESC`)
    /// and acquires a crash-recoverable visibility lease.
    pub fn dequeue_next(
        &self,
        utc_day: &str,
        lease_expires_epoch: i64,
    ) -> Result<Option<QueuedJob>, StoreError> {
        type RowTuple = (String, String, String, i64, String, f64, i64, f64);
        let row: Option<RowTuple> = self
            .store
            .connection()
            .query_row(
                "SELECT job_id, kind, lane, priority_tier, deadline_utc, voi_log, attempts, est_cost_usd
                 FROM fcast_jobs
                 WHERE state = 'QUEUED'
                 ORDER BY priority_tier DESC, deadline_utc ASC, voi_log DESC
                 LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            job_id,
            kind,
            lane,
            priority_tier,
            deadline_utc,
            voi_log,
            attempts,
            est_cost_usd,
        )) = row
        else {
            return Ok(None);
        };
        if est_cost_usd > 0.0 {
            self.store.check_budget_available(utc_day, est_cost_usd)?;
        }
        self.store.connection().execute(
            "UPDATE fcast_jobs SET state = 'RUNNING', lease_expires_epoch = ?1 WHERE job_id = ?2",
            params![lease_expires_epoch, &job_id],
        )?;
        Ok(Some(QueuedJob {
            job_id,
            kind,
            lane,
            priority_tier,
            deadline_utc,
            voi_log,
            attempts,
            est_cost_usd,
        }))
    }

    /// Reclaims stale `RUNNING` visibility leases after `SIGKILL` / daemon crash (`GATE-19`).
    pub fn reclaim_expired_leases(&self, now_epoch: i64) -> Result<usize, StoreError> {
        let n = self.store.connection().execute(
            "UPDATE fcast_jobs
             SET state = 'QUEUED', lease_expires_epoch = NULL
             WHERE state = 'RUNNING' AND lease_expires_epoch < ?1",
            params![now_epoch],
        )?;
        Ok(n)
    }

    /// Records a job failure; quarantines into `DEAD_LETTER` once `attempts >= MAX_JOB_ATTEMPTS` (`§13.2`).
    pub fn record_failure(&self, job_id: &str, err_msg: &str) -> Result<String, StoreError> {
        let attempts: i64 = self.store.connection().query_row(
            "SELECT attempts FROM fcast_jobs WHERE job_id = ?1",
            params![job_id],
            |r| r.get(0),
        )?;
        let next_attempts = attempts + 1;
        let next_state = if next_attempts >= MAX_JOB_ATTEMPTS {
            "DEAD_LETTER"
        } else {
            "QUEUED"
        };
        self.store.connection().execute(
            "UPDATE fcast_jobs
             SET attempts = ?1, state = ?2, lease_expires_epoch = NULL, last_error = ?3
             WHERE job_id = ?4",
            params![next_attempts, next_state, err_msg, job_id],
        )?;
        Ok(next_state.to_owned())
    }

    /// Publishes a coalesced domain event (`INSERT OR IGNORE` on `idempotency_key`, `§13.2`).
    pub fn publish_domain_event(
        &self,
        event_kind: &str,
        question_id: &str,
        forecast_id: Option<&str>,
        as_of_utc: &str,
        idempotency_key: &str,
        payload_json: &str,
    ) -> Result<bool, StoreError> {
        let n = self.store.connection().execute(
            "INSERT OR IGNORE INTO domain_events
             (event_kind, question_id, forecast_id, as_of_utc, idempotency_key, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event_kind,
                question_id,
                forecast_id,
                as_of_utc,
                idempotency_key,
                payload_json
            ],
        )?;
        Ok(n > 0)
    }

    /// Polls domain events strictly after `after_seq` in deterministic `event_seq ASC` order.
    pub fn poll_domain_events(&self, after_seq: i64) -> Result<Vec<DomainEventRecord>, StoreError> {
        let mut stmt = self.store.connection().prepare(
            "SELECT event_seq, event_kind, question_id, forecast_id, as_of_utc, idempotency_key, payload_json
             FROM domain_events
             WHERE event_seq > ?1
             ORDER BY event_seq ASC",
        )?;
        let mut rows = stmt.query(params![after_seq])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(DomainEventRecord {
                event_seq: r.get(0)?,
                event_kind: r.get(1)?,
                question_id: r.get(2)?,
                forecast_id: r.get(3)?,
                as_of_utc: r.get(4)?,
                idempotency_key: r.get(5)?,
                payload_json: r.get(6)?,
            });
        }
        Ok(out)
    }
}
