//! Axiomatic Split 2 (`FR4` — Central `RepRouter` & Two-Phase ReSHAPE Forecasting Planning).
//!
//! Strictly lower-triangular dependencies:
//! - Reads ONLY `IntakeSnapshot` (`FR3a`), external `PlanConstraintSpec`, `TimeBudget`,
//!   `BillingPacingTier`, and an immutable pre-committed `TuningCacheSnapshot(t-1)` from `Chronicle`.
//! - NEVER reads live mutable `Crucible` (`DP8`) or `Cockpit` (`DP10`) state.
//! - Emits `EvidenceAcquisitionPlan` (`FR3b`) for downstream execution.

use fcast_infra::daemon::{BillingPacingTier, TimeBudget, TimeBudgetMode};
use fcast_ingest::pipeline::{EvidenceAcquisitionPlan, IntakeSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Canonical representation library (`ALTERNATIVE_PLAN_V1.md` §II.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepresentationId {
    /// Microsecond conjugate Beta-logit filter or lightweight Tier-0 tick.
    R0ConjugateStream,
    /// K-way Dirichlet-Categorical simplex with fixed-point `>= 0.01` flooring.
    R1DirichletSimplex,
    /// 13-percentile multimodal `Cdf201` / Mantic `N <= 31` PMF with open-tail floor.
    R2DensityGrid,
    /// Three-Doc Memory Overlay (`Gold` + `Episodic` + `StateDoc`) + `fetch_raw_data` + `SEASONAL_NOTE`.
    R3ThreeDocOverlay,
    /// Causal `fnx-*` graph + CAS-handle PBM `fold` + Martingale `VoI_log`.
    R4CausalGraphFold,
    /// Mechanical staged-reveal `6D+2G` & binding Red-Team verification.
    R5StagedRevealAudit,
}

/// Empirical-Bayes cost & error statistics for one representation cell in `TuningCacheSnapshot`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TuningCellStats {
    pub sample_n: usize,
    pub mean_cost_usd: f64,
    pub mean_latency_sec: f64,
    /// Expected proper-score loss (e.g., Brier score in `[0, 1]`).
    pub mean_error_loss: f64,
    /// Standard error of `mean_error_loss`, driving cold-start LCB optimism.
    pub error_std: f64,
}

impl TuningCellStats {
    /// Lower Confidence Bound `E_LCB = max(0, E_hat - beta * sigma_E)` preventing cold-start
    /// collapse onto the cheapest representation (`R0`) while resolutions lag costs.
    #[must_use]
    pub fn error_lcb(&self, beta_ucb: f64) -> f64 {
        let explore_bonus = if self.sample_n < 20 {
            beta_ucb.max(1.0) * self.error_std.max(0.02)
        } else {
            beta_ucb.max(0.0) * self.error_std.max(0.0)
        };
        (self.mean_error_loss - explore_bonus).max(0.0)
    }
}

/// Immutable version-pinned Tuning Cache snapshot `(t-1)` loaded from `Chronicle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuningCacheSnapshot {
    pub snapshot_id: String,
    pub created_utc: String,
    pub resolved_n: usize,
    pub cells: BTreeMap<RepresentationId, TuningCellStats>,
}

impl TuningCacheSnapshot {
    /// Cold-start default snapshot with principled priors before any live resolutions arrive.
    #[must_use]
    pub fn cold_start() -> Self {
        let mut cells = BTreeMap::new();
        cells.insert(
            RepresentationId::R0ConjugateStream,
            TuningCellStats {
                sample_n: 0,
                mean_cost_usd: 0.02,
                mean_latency_sec: 8.0,
                mean_error_loss: 0.185,
                error_std: 0.030,
            },
        );
        cells.insert(
            RepresentationId::R3ThreeDocOverlay,
            TuningCellStats {
                sample_n: 0,
                mean_cost_usd: 0.11,
                mean_latency_sec: 42.0,
                mean_error_loss: 0.132,
                error_std: 0.045,
            },
        );
        cells.insert(
            RepresentationId::R4CausalGraphFold,
            TuningCellStats {
                sample_n: 0,
                mean_cost_usd: 0.24,
                mean_latency_sec: 85.0,
                mean_error_loss: 0.118,
                error_std: 0.055,
            },
        );
        Self {
            snapshot_id: "tuning_snap_cold_v0".into(),
            created_utc: "2026-11-26T00:00:00Z".into(),
            resolved_n: 0,
            cells,
        }
    }
}

/// External planning constraints supplied to `RepRouter` (working hypothesis soft caps + optional
/// Phase-A selection override). Never couples `RepRouter` back to `Cockpit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanConstraintSpec {
    pub soft_max_cost_usd: f64,
    pub soft_max_latency_sec: f64,
    /// Optional explicit user/operator selection from Phase A concrete trade-off cards.
    pub accepted_override_rep: Option<RepresentationId>,
    /// Allow autonomous soft-cap relaxation up to `1 + relax_factor` if Pareto-dominant.
    pub autonomous_relax_factor: f64,
}

impl Default for PlanConstraintSpec {
    fn default() -> Self {
        Self {
            soft_max_cost_usd: 0.10,
            soft_max_latency_sec: 60.0,
            accepted_override_rep: None,
            autonomous_relax_factor: 0.50,
        }
    }
}

/// One concrete candidate plan surfaced during Phase A (`ReSHAPE`, `$0` Tuning Cache preview).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateForecastPlan {
    pub rep: RepresentationId,
    pub est_cost_usd: f64,
    pub est_latency_sec: f64,
    pub est_error_lcb: f64,
    pub relaxes_soft_constraint: bool,
    pub lagrangian_score: f64,
}

/// Final Phase B single-shot plan emitted by `RepRouter` to drive `FR3b` (`Harvester`) & `FR5` (`Loom`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedForecastPlan {
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub selected_rep: Option<RepresentationId>,
    pub tuning_snapshot_id: String,
    pub reshaped_spec_hash: String,
    pub evidence_plan: Option<EvidenceAcquisitionPlan>,
}

/// Central Two-Phase ReSHAPE Representation Router (`FR4`).
#[derive(Debug, Clone, Copy, Default)]
pub struct RepRouter;

impl RepRouter {
    /// Shadow price `lambda` scaling cost penalty by `BillingPacingTier` and tournament `prize_weight`.
    #[must_use]
    pub fn shadow_price_lambda(pacing: BillingPacingTier, prize_weight: f64) -> f64 {
        let base = match pacing {
            BillingPacingTier::Full => 0.15,
            BillingPacingTier::Economy => 0.45,
            BillingPacingTier::Minimal => 1.25,
            BillingPacingTier::Stop => f64::INFINITY,
        };
        base / prize_weight.max(0.25)
    }

    /// **Phase A (`ReSHAPE`):** Generates concrete boundary-probing candidate plans from the
    /// immutable `TuningCacheSnapshot(t-1)` at `$0` LLM spend — including soft-relaxed trade-offs.
    #[must_use]
    pub fn phase_a_reshape(
        intake: &IntakeSnapshot,
        spec: &PlanConstraintSpec,
        time_budget: &TimeBudget,
        pacing: BillingPacingTier,
        snapshot: &TuningCacheSnapshot,
    ) -> Vec<CandidateForecastPlan> {
        if time_budget.should_skip() || !pacing.allows_dispatch() {
            return Vec::new();
        }
        let lambda = Self::shadow_price_lambda(pacing, intake.prize_weight);
        let max_relaxed_cost =
            spec.soft_max_cost_usd * (1.0 + spec.autonomous_relax_factor.max(0.0));
        let max_relaxed_lat =
            spec.soft_max_latency_sec * (1.0 + spec.autonomous_relax_factor.max(0.0));

        let mut candidates = Vec::new();
        for (&rep, stats) in &snapshot.cells {
            // Hard feasibility guards: FastPath or Minimal restricts to single-pass R0.
            if (matches!(time_budget.mode, TimeBudgetMode::FastPath)
                || matches!(pacing, BillingPacingTier::Minimal))
                && rep != RepresentationId::R0ConjugateStream
            {
                continue;
            }
            let exceeds_soft = stats.mean_cost_usd > spec.soft_max_cost_usd
                || stats.mean_latency_sec > spec.soft_max_latency_sec;
            if exceeds_soft
                && (stats.mean_cost_usd > max_relaxed_cost
                    || stats.mean_latency_sec > max_relaxed_lat)
            {
                continue;
            }
            let e_lcb = stats.error_lcb(1.0);
            let score = e_lcb + lambda * stats.mean_cost_usd;
            candidates.push(CandidateForecastPlan {
                rep,
                est_cost_usd: stats.mean_cost_usd,
                est_latency_sec: stats.mean_latency_sec,
                est_error_lcb: e_lcb,
                relaxes_soft_constraint: exceeds_soft,
                lagrangian_score: score,
            });
        }
        candidates.sort_by(|a, b| a.lagrangian_score.total_cmp(&b.lagrangian_score));
        candidates
    }

    /// **Phase B (Single-Shot Selection):** Picks the optimal representation once from the
    /// reshaped feasible set and emits a concrete `EvidenceAcquisitionPlan` (`FR3b`).
    #[must_use]
    pub fn phase_b_select(
        intake: &IntakeSnapshot,
        as_of_utc: &str,
        spec: &PlanConstraintSpec,
        time_budget: &TimeBudget,
        pacing: BillingPacingTier,
        snapshot: &TuningCacheSnapshot,
    ) -> SelectedForecastPlan {
        let candidates = Self::phase_a_reshape(intake, spec, time_budget, pacing, snapshot);
        if candidates.is_empty() {
            let reason = if !pacing.allows_dispatch() {
                "billing_pacing_stop"
            } else if time_budget.skip_after_spot_scoring_time {
                "skip_after_spot_scoring_time"
            } else {
                "time_budget_intake_skip"
            };
            return SelectedForecastPlan {
                skipped: true,
                skip_reason: Some(reason.into()),
                selected_rep: None,
                tuning_snapshot_id: snapshot.snapshot_id.clone(),
                reshaped_spec_hash: "skipped".into(),
                evidence_plan: None,
            };
        }

        let chosen = spec
            .accepted_override_rep
            .and_then(|ov| candidates.iter().find(|c| c.rep == ov))
            .unwrap_or(&candidates[0]);

        let dual = matches!(
            chosen.rep,
            RepresentationId::R3ThreeDocOverlay | RepresentationId::R4CausalGraphFold
        ) && matches!(pacing, BillingPacingTier::Full);
        let zero_key = matches!(pacing, BillingPacingTier::Minimal)
            || chosen.rep == RepresentationId::R0ConjugateStream;

        let evidence_plan = EvidenceAcquisitionPlan {
            question_id: intake.question_id.clone(),
            as_of_utc: as_of_utc.to_owned(),
            fetch_raw_data: !zero_key,
            attach_seasonal_note: true,
            dual_independent_reports: dual,
            zero_key_fallback: zero_key,
        };

        let hash_input = format!(
            "{}:{}:{:?}:{:.4}",
            snapshot.snapshot_id, intake.question_id, chosen.rep, chosen.lagrangian_score
        );
        let reshaped_spec_hash = format!(
            "plan_{}",
            &blake3::hash(hash_input.as_bytes()).to_hex()[0..16]
        );

        SelectedForecastPlan {
            skipped: false,
            skip_reason: None,
            selected_rep: Some(chosen.rep),
            tuning_snapshot_id: snapshot.snapshot_id.clone(),
            reshaped_spec_hash,
            evidence_plan: Some(evidence_plan),
        }
    }
}
