//! `fcast-bench` (DAG Layer L4 — `SUB-05` Crucible).
//!
//! Hosts `QuestionMetaForecast`, full-universe 10-metric anti-selection-bias scoring,
//! `FutureEval` / `MiniBench` & `ForecastBench` solvers, and air-gapped `GEPA` replay (`M5`).

use serde::{Deserialize, Serialize};

/// Exhaustive four-way eligible universe disposition at horizon `H` (`§12.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniverseDisposition {
    Issued,
    Excluded,
    Abandoned,
    QueuedMissed,
    Ineligible,
}
