//! Milestone `M2` Vintage-Locked `DataPacks` (`GATE-07`), Horvitz-Thompson right-censoring
//! (`< 0.30` completeness), `fetch_raw_data` CSV/JSON parsing with fail-closed `[PARTIAL_CONTEXT]`
//! guard (`elicitation-measurement` Finding #5), `3–5 yr` `SEASONAL_NOTE` synthesis, and
//! pre-LLM `PersistenceAndDriftBaseline` (`thesis_model_candidate_v1`).

use fcast_core::time::{is_utc_le, parse_utc_epoch_ms};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Horvitz-Thompson completeness floor below which a series is flagged as right-censored (`GATE-07`).
pub const RIGHT_CENSORED_COMPLETENESS_THRESHOLD: f64 = 0.30;

/// Default raw-table byte ceiling before refusing silent truncation (`[PARTIAL_CONTEXT]`).
pub const DEFAULT_MAX_RAW_TABLE_BYTES: usize = 512 * 1024;

#[derive(Debug, Error, PartialEq)]
pub enum DataPackError {
    #[error(
        "VINTAGE_FENCE_VIOLATION for {series_id}: available_utc={available_utc} > as_of_utc={as_of_utc}"
    )]
    AvailableAfterCutoff {
        series_id: String,
        available_utc: String,
        as_of_utc: String,
    },
    #[error(
        "ALFRED_VINTAGE_HEADER_VIOLATION for {series_id}: returned header_vintage_utc={header_vintage_utc} > requested_vintage_utc={requested_vintage_utc}"
    )]
    HeaderVintageAfterRequested {
        series_id: String,
        header_vintage_utc: String,
        requested_vintage_utc: String,
    },
    #[error("invalid UTC timestamp in DataPack: {0}")]
    InvalidTimestamp(String),
    #[error("non-finite numeric value in series {series_id} at period {period}")]
    NonFiniteValue { series_id: String, period: String },
    #[error("invalid CSV/JSON raw table payload: {0}")]
    ParseError(String),
    #[error("series_id mismatch: expected {expected}, got {actual}")]
    SeriesMismatch { expected: String, actual: String },
    #[error("invalid reporting probability pi={0} (must be in (0, 1] with finite 1/pi sum)")]
    InvalidReportingProb(f64),
}

/// Fail-closed completeness tag for raw documents/tables fed into `Loom`
/// (`elicitation-measurement` Finding #5: incomplete primary text degrades accuracy below plain summaries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ContextCompleteness {
    /// Complete primary table/document captured without truncation.
    Complete,
    /// Truncated, partial, or missing conforming sections — MUST trigger `THIN_PUBLISH_FLOOR`.
    Partial { reason: String },
}

impl ContextCompleteness {
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// One vintage-locked official observation with explicit ALFRED/official returned header vintage
/// (never trusting query echo alone — Thesis postmortem #1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataPackObservation {
    pub series_id: String,
    pub period: String,
    pub value: f64,
    pub event_utc: Option<String>,
    /// Public ingestion availability timestamp (`available_utc <= as_of_utc`).
    pub available_utc: String,
    /// Returned upstream vintage header (`header_vintage_utc <= requested_vintage_utc`).
    pub header_vintage_utc: String,
}

/// Explicit immutable release-schedule anchor preventing recursive `+7d` window drift
/// (Thesis `weekly-claims-release-window-drift.md` postmortem).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseScheduleAnchor {
    pub series_id: String,
    pub official_release_utc: String,
    pub holiday_shifted: bool,
}

/// Vintage-locked `DataPack` bundle (`GATE-07`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VintageDataPack {
    pub series_id: String,
    pub as_of_utc: String,
    pub requested_vintage_utc: String,
    pub observations: Vec<DataPackObservation>,
    pub completeness: ContextCompleteness,
}

impl VintageDataPack {
    /// Builds a strictly time-fenced `VintageDataPack`, enforcing:
    /// - `requested_vintage_utc <= as_of_utc`
    /// - `obs.available_utc <= as_of_utc`
    /// - `obs.header_vintage_utc <= min(requested_vintage_utc, as_of_utc)`
    /// - `obs.event_utc <= as_of_utc` (when present)
    /// - `obs.series_id == series_id`
    /// - deterministic chronological ordering by `(available_utc_ms, period)`.
    pub fn build_fenced(
        series_id: impl Into<String>,
        as_of_utc: impl Into<String>,
        requested_vintage_utc: impl Into<String>,
        observations: Vec<DataPackObservation>,
        completeness: ContextCompleteness,
    ) -> Result<Self, DataPackError> {
        let series_id = series_id.into();
        let as_of_utc = as_of_utc.into();
        let requested_vintage_utc = requested_vintage_utc.into();
        parse_utc_epoch_ms(&as_of_utc)
            .map_err(|_| DataPackError::InvalidTimestamp(as_of_utc.clone()))?;
        parse_utc_epoch_ms(&requested_vintage_utc)
            .map_err(|_| DataPackError::InvalidTimestamp(requested_vintage_utc.clone()))?;

        if !is_utc_le(&requested_vintage_utc, &as_of_utc) {
            return Err(DataPackError::HeaderVintageAfterRequested {
                series_id,
                header_vintage_utc: requested_vintage_utc,
                requested_vintage_utc: as_of_utc,
            });
        }

        let mut keyed: Vec<(i64, DataPackObservation)> = Vec::with_capacity(observations.len());
        for obs in observations {
            if obs.series_id != series_id {
                return Err(DataPackError::SeriesMismatch {
                    expected: series_id,
                    actual: obs.series_id,
                });
            }
            if !obs.value.is_finite() {
                return Err(DataPackError::NonFiniteValue {
                    series_id: obs.series_id,
                    period: obs.period,
                });
            }
            let avail_ms = parse_utc_epoch_ms(&obs.available_utc)
                .map_err(|_| DataPackError::InvalidTimestamp(obs.available_utc.clone()))?;
            parse_utc_epoch_ms(&obs.header_vintage_utc)
                .map_err(|_| DataPackError::InvalidTimestamp(obs.header_vintage_utc.clone()))?;

            if let Some(ev_ts) = &obs.event_utc {
                parse_utc_epoch_ms(ev_ts)
                    .map_err(|_| DataPackError::InvalidTimestamp(ev_ts.clone()))?;
                if !is_utc_le(ev_ts, &as_of_utc) {
                    return Err(DataPackError::AvailableAfterCutoff {
                        series_id: obs.series_id,
                        available_utc: ev_ts.clone(),
                        as_of_utc,
                    });
                }
            }

            if !is_utc_le(&obs.available_utc, &as_of_utc) {
                return Err(DataPackError::AvailableAfterCutoff {
                    series_id: obs.series_id,
                    available_utc: obs.available_utc,
                    as_of_utc,
                });
            }
            if !is_utc_le(&obs.header_vintage_utc, &requested_vintage_utc)
                || !is_utc_le(&obs.header_vintage_utc, &as_of_utc)
            {
                return Err(DataPackError::HeaderVintageAfterRequested {
                    series_id: obs.series_id,
                    header_vintage_utc: obs.header_vintage_utc,
                    requested_vintage_utc,
                });
            }
            keyed.push((avail_ms, obs));
        }
        keyed.sort_by(|(ms_a, a), (ms_b, b)| ms_a.cmp(ms_b).then_with(|| a.period.cmp(&b.period)));
        let verified = keyed.into_iter().map(|(_, o)| o).collect();

        Ok(Self {
            series_id,
            as_of_utc,
            requested_vintage_utc,
            observations: verified,
            completeness,
        })
    }

    /// Extracts chronological numeric values from the verified observations.
    #[must_use]
    pub fn values(&self) -> Vec<f64> {
        self.observations.iter().map(|o| o.value).collect()
    }
}

/// Horvitz-Thompson right-censoring diagnostic (`GATE-07`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorvitzThompsonReport {
    pub observed_count: usize,
    pub mean_reporting_completeness: f64,
    pub ht_adjusted_total: f64,
    /// `true` when `mean_reporting_completeness < 0.30` (signals high reporting lag / right-censoring).
    pub right_censored: bool,
}

/// Evaluates reporting completeness over a slice of per-event inclusion probabilities `pi_i ∈ (0, 1]`.
pub fn horvitz_thompson_completeness(
    reporting_probs: &[f64],
) -> Result<HorvitzThompsonReport, DataPackError> {
    if reporting_probs.is_empty() {
        return Ok(HorvitzThompsonReport {
            observed_count: 0,
            mean_reporting_completeness: 0.0,
            ht_adjusted_total: 0.0,
            right_censored: true,
        });
    }
    let mut sum_pi = 0.0_f64;
    let mut ht_total = 0.0_f64;
    for &pi in reporting_probs {
        if !pi.is_finite() || pi <= 0.0 || pi > 1.0 {
            return Err(DataPackError::InvalidReportingProb(pi));
        }
        sum_pi += pi;
        ht_total += 1.0 / pi;
        if !ht_total.is_finite() {
            return Err(DataPackError::InvalidReportingProb(pi));
        }
    }
    let mean_pi = sum_pi / (reporting_probs.len() as f64);
    Ok(HorvitzThompsonReport {
        observed_count: reporting_probs.len(),
        mean_reporting_completeness: mean_pi,
        ht_adjusted_total: ht_total,
        right_censored: mean_pi < RIGHT_CENSORED_COMPLETENESS_THRESHOLD,
    })
}

/// Deterministic `3–5 yr` calendar-window `SEASONAL_NOTE` (`+5.5 pts/Q` upgrade, Plan v2.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeasonalNote {
    pub series_id: String,
    pub calendar_window: String,
    pub sample_n: usize,
    pub median_value: f64,
    pub p10_value: f64,
    pub p90_value: f64,
    pub summary: String,
}

/// Synthesizes a `SEASONAL_NOTE` from pre-cutoff values belonging to the target calendar window.
#[must_use]
pub fn build_seasonal_note(
    series_id: &str,
    calendar_window: &str,
    window_values: &[f64],
) -> Option<SeasonalNote> {
    let mut vals: Vec<f64> = window_values
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    if vals.is_empty() {
        return None;
    }
    vals.sort_by(f64::total_cmp);
    let p10 = linear_quantile(&vals, 0.10);
    let median = linear_quantile(&vals, 0.50);
    let p90 = linear_quantile(&vals, 0.90);
    let summary = format!(
        "SEASONAL_NOTE[{series_id} @ {calendar_window}, n={}]: median={median:.4}, 80%-band=[{p10:.4}, {p90:.4}]",
        vals.len()
    );
    Some(SeasonalNote {
        series_id: series_id.to_owned(),
        calendar_window: calendar_window.to_owned(),
        sample_n: vals.len(),
        median_value: median,
        p10_value: p10,
        p90_value: p90,
        summary,
    })
}

/// Interval estimation method used by [`PersistenceAndDriftBaseline`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineIntervalMethod {
    ResidualQuantile,
    FallbackPrior,
}

/// Pre-LLM statistical persistence baseline (`thesis_model_candidate_v1`, Thesis Pass 2 Upgrade #2).
///
/// Computed purely from pre-cutoff official observations BEFORE any LLM call, supplying an
/// explicit override bar (`override_bar_width = 0.25 * (p90 - p10)`) that the analyst must
/// justify overcoming with a named current signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistenceAndDriftBaseline {
    pub series_id: String,
    pub train_cutoff_utc: String,
    pub calibration_n: usize,
    pub point: f64,
    pub p10: f64,
    pub p50: f64,
    pub p90: f64,
    pub interval_method: BaselineIntervalMethod,
    pub walk_forward_mae: Option<f64>,
    pub override_bar_width: f64,
}

/// Builds a pure-Rust last-print persistence baseline with empirical one-step residual quantiles.
#[must_use]
pub fn build_persistence_baseline(
    series_id: &str,
    train_cutoff_utc: &str,
    history: &[f64],
) -> Option<PersistenceAndDriftBaseline> {
    if history.is_empty() || history.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let last = *history.last()?;
    let mut diffs: Vec<f64> = history.windows(2).map(|w| w[1] - w[0]).collect();

    let (p10, p90, method, mae) = if diffs.len() >= 2 {
        let mean_abs = diffs.iter().map(|d| d.abs()).sum::<f64>() / (diffs.len() as f64);
        diffs.sort_by(f64::total_cmp);
        let q10 = linear_quantile(&diffs, 0.10);
        let q90 = linear_quantile(&diffs, 0.90);
        let mut lo = (last + q10).min(last);
        let mut hi = (last + q90).max(last);
        if (hi - lo).abs() <= 1e-12 {
            let pad = (last.abs() * 0.05).max(1e-3);
            lo = last - pad;
            hi = last + pad;
        }
        (
            lo,
            hi,
            BaselineIntervalMethod::ResidualQuantile,
            Some(mean_abs),
        )
    } else {
        let scale = (last.abs() * 0.10).max(1.0);
        (
            last - scale,
            last + scale,
            BaselineIntervalMethod::FallbackPrior,
            None,
        )
    };

    Some(PersistenceAndDriftBaseline {
        series_id: series_id.to_owned(),
        train_cutoff_utc: train_cutoff_utc.to_owned(),
        calibration_n: history.len(),
        point: last,
        p10,
        p50: last,
        p90,
        interval_method: method,
        walk_forward_mae: mae,
        override_bar_width: 0.25 * (p90 - p10),
    })
}

/// Parsed raw table output (`fetch_raw_data` CSV/JSON reader, `+7.0 pts/Q`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedRawTable {
    pub series_id: String,
    pub rows: Vec<DataPackObservation>,
    pub completeness: ContextCompleteness,
}

/// Parses a `period,value,available_utc,header_vintage_utc` RFC-4180 CSV payload without silently
/// truncating. Enforces all 4 header column names, strips UTF-8 BOM, rejects non-finite numbers,
/// refuses unbounded DoS payloads (`> max(4 * max_bytes, 4 MiB)`), and tags oversized or empty
/// tables as [`ContextCompleteness::Partial`] so `PreLoomGuard` forces `THIN_PUBLISH_FLOOR`.
pub fn parse_raw_csv_series(
    series_id: &str,
    csv_text: &str,
    max_bytes: usize,
) -> Result<ParsedRawTable, DataPackError> {
    let hard_cap = max_bytes.saturating_mul(4).max(4 * 1024 * 1024);
    if csv_text.len() > hard_cap {
        return Err(DataPackError::ParseError(format!(
            "CSV payload ({} bytes) exceeded hard memory cap ({hard_cap} bytes)",
            csv_text.len()
        )));
    }
    let clean = csv_text.strip_prefix('\u{FEFF}').unwrap_or(csv_text);
    let mut completeness = if clean.len() > max_bytes {
        ContextCompleteness::Partial {
            reason: format!(
                "raw CSV payload ({} bytes) exceeded complete-context cap ({max_bytes} bytes)",
                clean.len()
            ),
        }
    } else {
        ContextCompleteness::Complete
    };

    let mut lines = clean.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else {
        return Err(DataPackError::ParseError("empty CSV payload".into()));
    };
    let cols = parse_csv_fields(header)?;
    if cols.len() < 4
        || cols[0] != "period"
        || cols[1] != "value"
        || cols[2] != "available_utc"
        || cols[3] != "header_vintage_utc"
    {
        return Err(DataPackError::ParseError(format!(
            "unexpected CSV header (expected period,value,available_utc,header_vintage_utc): {header:?}"
        )));
    }

    let mut rows = Vec::new();
    for (idx, line) in lines.enumerate() {
        let fields = parse_csv_fields(line)?;
        if fields.len() < 4 {
            return Err(DataPackError::ParseError(format!(
                "CSV row {} has fewer than 4 columns",
                idx + 2
            )));
        }
        let value: f64 = fields[1].parse().map_err(|_| {
            DataPackError::ParseError(format!("invalid float {:?} on row {}", fields[1], idx + 2))
        })?;
        if !value.is_finite() {
            return Err(DataPackError::NonFiniteValue {
                series_id: series_id.to_owned(),
                period: fields[0].clone(),
            });
        }
        rows.push(DataPackObservation {
            series_id: series_id.to_owned(),
            period: fields[0].clone(),
            value,
            event_utc: None,
            available_utc: fields[2].clone(),
            header_vintage_utc: fields[3].clone(),
        });
    }
    if rows.is_empty() && completeness.is_complete() {
        completeness = ContextCompleteness::Partial {
            reason: "CSV payload contained header only (0 observations)".into(),
        };
    }
    Ok(ParsedRawTable {
        series_id: series_id.to_owned(),
        rows,
        completeness,
    })
}

/// Parses a JSON array of `DataPackObservation` records with fail-closed completeness tagging.
pub fn parse_raw_json_series(
    series_id: &str,
    json_text: &str,
    max_bytes: usize,
) -> Result<ParsedRawTable, DataPackError> {
    let hard_cap = max_bytes.saturating_mul(4).max(4 * 1024 * 1024);
    if json_text.len() > hard_cap {
        return Err(DataPackError::ParseError(format!(
            "JSON payload ({} bytes) exceeded hard memory cap ({hard_cap} bytes)",
            json_text.len()
        )));
    }
    let clean = json_text.strip_prefix('\u{FEFF}').unwrap_or(json_text);
    let mut completeness = if clean.len() > max_bytes {
        ContextCompleteness::Partial {
            reason: format!(
                "raw JSON payload ({} bytes) exceeded complete-context cap ({max_bytes} bytes)",
                clean.len()
            ),
        }
    } else {
        ContextCompleteness::Complete
    };
    let rows: Vec<DataPackObservation> =
        serde_json::from_str(clean).map_err(|e| DataPackError::ParseError(e.to_string()))?;
    for row in &rows {
        if !row.value.is_finite() {
            return Err(DataPackError::NonFiniteValue {
                series_id: row.series_id.clone(),
                period: row.period.clone(),
            });
        }
    }
    if rows.is_empty() && completeness.is_complete() {
        completeness = ContextCompleteness::Partial {
            reason: "JSON payload contained 0 observations".into(),
        };
    }
    Ok(ParsedRawTable {
        series_id: series_id.to_owned(),
        rows,
        completeness,
    })
}

fn parse_csv_fields(line: &str) -> Result<Vec<String>, DataPackError> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    let _ = chars.next();
                    cur.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(ch);
            }
        } else {
            match ch {
                '"' if cur.trim().is_empty() => {
                    cur.clear();
                    in_quotes = true;
                }
                ',' => {
                    fields.push(cur.trim().to_owned());
                    cur.clear();
                }
                other => cur.push(other),
            }
        }
    }
    if in_quotes {
        return Err(DataPackError::ParseError("unterminated CSV quote".into()));
    }
    fields.push(cur.trim().to_owned());
    Ok(fields)
}

fn linear_quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = q.clamp(0.0, 1.0) * ((sorted.len() - 1) as f64);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - (lo as f64);
    sorted[lo] + frac * (sorted[hi] - sorted[lo])
}
