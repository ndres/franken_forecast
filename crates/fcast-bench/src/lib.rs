//! `fcast-bench` (DAG Layer L4 — `SUB-05` Crucible).
//!
//! Hosts `QuestionMetaForecast`, full-universe 10-metric anti-selection-bias scoring (`GATE-06`),
//! Thesis Lab `pairedNormalizedCrpsDelta` with `1e-12` relative dispersion floor, `M1.5`
//! end-to-end vertical-slice demo pipeline, `FutureEval` / `MiniBench` & `ForecastBench` solvers.

pub mod cohort;
pub mod scoring;

use serde::{Deserialize, Serialize};

pub use cohort::{
    BenchmarkOutcome, ResolvedBenchmarkQuestion, deterministic_m15_demo_cohort, load_cohort_json,
};
pub use scoring::{
    BenchError, ClusteredBootstrapCi, CounterfactualDoseProbe, FullUniverseScorecard,
    M15DemoReport, NORMALIZATION_SCALE_RELATIVE_FLOOR, adaptive_ece, brier_score,
    compute_history_step_scale, evaluate_arm_from_store, exact_sign_flip_p_value, gaussian_crps,
    log_score, paired_normalized_crps_delta, release_date_clustered_bootstrap_ci,
    run_m15_demo_pipeline,
};

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
