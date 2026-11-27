-- FrankenForecast (fcast) Migration 0006: Operational query indexes for job dequeue,
-- lease reclamation, horizon projections, and disjoint resolution joins.

CREATE INDEX IF NOT EXISTS idx_fcast_jobs_dequeue
    ON fcast_jobs (state, priority_tier DESC, deadline_utc ASC, voi_log DESC);

CREATE INDEX IF NOT EXISTS idx_fcast_jobs_running_lease
    ON fcast_jobs (state, lease_expires_epoch);

CREATE INDEX IF NOT EXISTS idx_forecast_revisions_q_asof
    ON forecast_revisions (question_id, as_of_utc);

CREATE INDEX IF NOT EXISTS idx_forecast_revisions_fc_seq
    ON forecast_revisions (forecast_id, seq_no);

CREATE INDEX IF NOT EXISTS idx_question_resolutions_q
    ON question_resolutions (question_id, recorded_available_utc);
