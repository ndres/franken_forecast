-- FrankenForecast (fcast) Migration 0002: Optional King-Zeng cyber prior-shift metadata column
-- on forecast_streams (preserving strict immutability of historical forecast_revisions rows!).

ALTER TABLE forecast_streams ADD COLUMN cyber_prior_shift_nats REAL;
