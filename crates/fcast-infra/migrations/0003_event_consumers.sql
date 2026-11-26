-- FrankenForecast (fcast) Migration 0003: Durable event_consumers cursors over
-- transactional domain_events outbox (§13.2, Plan v2.1).

CREATE TABLE IF NOT EXISTS event_consumers (
    consumer_id TEXT PRIMARY KEY,
    last_acked_event_seq INTEGER NOT NULL,
    updated_utc TEXT NOT NULL
);
