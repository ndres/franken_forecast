-- FrankenForecast (fcast) Migration 0005: Append-only immutable RepRouter Tuning Cache Snapshots
-- (Axiomatic Split 2: RepRouter(t) reads only immutable TuningCacheSnapshot(t-1), never live Crucible state).

CREATE TABLE IF NOT EXISTS rep_router_tuning_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    created_utc TEXT NOT NULL,
    resolved_n INTEGER NOT NULL CHECK (resolved_n >= 0),
    payload_json TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS trg_tuning_snapshots_no_update
BEFORE UPDATE ON rep_router_tuning_snapshots
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_TUNING_CACHE_SNAPSHOT');
END;

CREATE TRIGGER IF NOT EXISTS trg_tuning_snapshots_no_delete
BEFORE DELETE ON rep_router_tuning_snapshots
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_TUNING_CACHE_SNAPSHOT');
END;
