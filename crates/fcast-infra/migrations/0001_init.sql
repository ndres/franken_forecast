-- FrankenForecast (fcast) Migration 0001: Canonical ForecastRevision, 3-Document Question Memory,
-- Full-Universe Anti-Selection-Bias Ledger, DurableJobStore, and Transactional domain_events Outbox.

CREATE TABLE IF NOT EXISTS _fcast_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    sha256_hex TEXT NOT NULL,
    applied_utc TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS questions (
    question_id TEXT PRIMARY KEY,
    venue TEXT NOT NULL,
    title TEXT NOT NULL,
    contract_sha256 TEXT NOT NULL,
    horizon_geometry TEXT NOT NULL CHECK (horizon_geometry IN ('point_in_time', 'rolling_window', 'cumulative_stopping_time')),
    created_utc TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS evidence_vault_index (
    ev_handle TEXT PRIMARY KEY,
    sha256_hex TEXT NOT NULL,
    span_id TEXT NOT NULL,
    available_utc TEXT NOT NULL,
    byte_len INTEGER NOT NULL,
    zstd_compressed INTEGER NOT NULL CHECK (zstd_compressed IN (0, 1))
);

CREATE TABLE IF NOT EXISTS forecast_streams (
    forecast_id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    stream_kind TEXT NOT NULL CHECK (stream_kind IN ('live_tournament', 'calibration_replay', 'manual_sandbox')),
    origin_revision_id TEXT
);

CREATE TABLE IF NOT EXISTS forecast_revisions (
    revision_id TEXT PRIMARY KEY,
    forecast_id TEXT NOT NULL REFERENCES forecast_streams(forecast_id),
    seq_no INTEGER NOT NULL,
    parent_revision_id TEXT REFERENCES forecast_revisions(revision_id),
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    as_of_utc TEXT NOT NULL,
    p_indep REAL NOT NULL CHECK (p_indep > 0.0 AND p_indep < 1.0),
    p_market REAL CHECK (p_market IS NULL OR (p_market > 0.0 AND p_market < 1.0)),
    probability_distribution TEXT NOT NULL,
    evidence_handles TEXT NOT NULL,
    model_version TEXT NOT NULL,
    calibration_version TEXT NOT NULL,
    rationale_handle TEXT NOT NULL REFERENCES evidence_vault_index(ev_handle),
    resolution_status TEXT NOT NULL CHECK (resolution_status IN ('unresolved', 'resolved', 'invalidated')),
    payload_schema_ver INTEGER NOT NULL DEFAULT 1,
    CHECK ((seq_no = 0 AND parent_revision_id IS NULL) OR (seq_no > 0 AND parent_revision_id IS NOT NULL)),
    UNIQUE(forecast_id, seq_no)
);

CREATE TRIGGER IF NOT EXISTS trg_forecast_revisions_no_update
BEFORE UPDATE ON forecast_revisions
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_FORECAST_REVISION');
END;

CREATE TRIGGER IF NOT EXISTS trg_forecast_revisions_no_delete
BEFORE DELETE ON forecast_revisions
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_FORECAST_REVISION');
END;

CREATE TABLE IF NOT EXISTS question_resolutions (
    resolution_id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    final_status TEXT NOT NULL CHECK (final_status IN ('resolved', 'invalidated')),
    outcome_payload_json TEXT,
    event_utc TEXT,
    resolved_published_utc TEXT NOT NULL,
    recorded_available_utc TEXT NOT NULL,
    adjudication_ev_handle TEXT REFERENCES evidence_vault_index(ev_handle),
    supersedes_resolution_id TEXT REFERENCES question_resolutions(resolution_id)
);

-- Three-Document Question Memory Overlay (§7.3)
CREATE TABLE IF NOT EXISTS question_gold_dossiers (
    question_id TEXT PRIMARY KEY REFERENCES questions(question_id),
    gold_dossier_handle TEXT NOT NULL REFERENCES evidence_vault_index(ev_handle),
    frozen_at_utc TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS episodic_events (
    event_id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    event_utc TEXT,
    published_utc TEXT NOT NULL,
    available_utc TEXT NOT NULL,
    ev_handle TEXT NOT NULL REFERENCES evidence_vault_index(ev_handle),
    summary TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS state_doc_revisions (
    state_rev_id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    seq_no INTEGER NOT NULL,
    parent_state_rev_id TEXT REFERENCES state_doc_revisions(state_rev_id),
    state_ev_handle TEXT NOT NULL REFERENCES evidence_vault_index(ev_handle),
    token_count INTEGER NOT NULL CHECK (token_count <= 4000),
    as_of_utc TEXT NOT NULL,
    UNIQUE(question_id, seq_no)
);

-- Irreversible Full Eligible Universe Anti-Selection-Bias Ledger (§12.3, INV-13)
CREATE TABLE IF NOT EXISTS universe_question_ledger (
    question_id TEXT PRIMARY KEY REFERENCES questions(question_id),
    discovered_utc TEXT NOT NULL,
    is_eligible INTEGER NOT NULL CHECK (is_eligible IN (0, 1)),
    est_cost_usd_p50 REAL NOT NULL,
    est_latency_sec_p50 REAL NOT NULL,
    popularity_score REAL NOT NULL CHECK (popularity_score BETWEEN 0.0 AND 100.0),
    complexity_score REAL NOT NULL CHECK (complexity_score BETWEEN 0.0 AND 100.0),
    disposition TEXT NOT NULL CHECK (disposition IN ('issued', 'excluded', 'abandoned', 'queued_missed', 'ineligible')),
    reason_code TEXT,
    total_cost_usd REAL NOT NULL DEFAULT 0.0,
    latency_sec REAL,
    crowd_prior REAL
);

CREATE TRIGGER IF NOT EXISTS trg_universe_eligible_irreversible
BEFORE UPDATE ON universe_question_ledger
WHEN OLD.is_eligible = 1 AND NEW.is_eligible = 0
BEGIN
    SELECT RAISE(ABORT, 'IRREVERSIBLE_ELIGIBLE_UNIVERSE_ADMISSION');
END;

-- Metronome DurableJobStore & Daily Spend Circuit Breaker (§13)
CREATE TABLE IF NOT EXISTS fcast_jobs (
    job_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    lane TEXT NOT NULL,
    priority_tier INTEGER NOT NULL DEFAULT 2,
    deadline_utc TEXT NOT NULL,
    voi_log REAL NOT NULL DEFAULT 0.0,
    idempotency_key TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELLED', 'DEAD_LETTER')),
    lease_expires_epoch INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    est_cost_usd REAL NOT NULL DEFAULT 0.0,
    last_error TEXT
);

CREATE TABLE IF NOT EXISTS daemon_budget_ledger (
    utc_day TEXT PRIMARY KEY,
    spent_usd REAL NOT NULL DEFAULT 0.0,
    cap_usd REAL NOT NULL DEFAULT 5.0
);

-- Transactional Cross-Subsystem Coordination Outbox (§13.2)
CREATE TABLE IF NOT EXISTS domain_events (
    event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_kind TEXT NOT NULL,
    question_id TEXT NOT NULL,
    forecast_id TEXT,
    as_of_utc TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    payload_json TEXT NOT NULL
);
