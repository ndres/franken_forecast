//! `fcast-core` (DAG Layer L0 — Zero I/O, Zero Net, Zero SQL).
//!
//! Houses the canonical immutable 12-field `ForecastRevision` (`KA-00`),
//! Two-Speed `(μ_t, N_t)` belief math, cross-question coherence & `MetaculusCdfBuilder`,
//! and pre-seal `MarketBlindingScrubber`.

pub mod belief;
pub mod blinding;
pub mod coherence;
pub mod forecast;
pub mod time;

/// Canonical numerical clamp epsilon (`1e-6`, `§7.1` & `lean_experiments/Claim05Spec.lean`).
pub const PROB_EPS: f64 = 1e-6;
