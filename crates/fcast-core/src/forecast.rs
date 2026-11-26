//! Canonical 12-field immutable `ForecastRevision` (`KA-00`, `§7.1`) and 4-stage lifecycle types.

use crate::PROB_EPS;
use crate::blinding::VerifiedEvidenceRef;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ForecastError {
    #[error("probability {0} is non-finite or outside (0, 1)")]
    InvalidProbability(f64),
    #[error("categorical distribution invalid: {0}")]
    InvalidCategorical(&'static str),
    #[error("cdf201 distribution invalid: {0}")]
    InvalidCdf(&'static str),
    #[error("lifecycle invariant violated: {0}")]
    LifecycleInvariant(&'static str),
    #[error("attribution waterfall residual {0:.3e} exceeds 1e-10")]
    WaterfallResidual(f64),
}

/// Strictly clamped probability in `[1e-6, 1 - 1e-6]`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProbClamped(f64);

impl ProbClamped {
    pub fn new(p: f64) -> Result<Self, ForecastError> {
        if !p.is_finite() || p <= 0.0 || p >= 1.0 {
            return Err(ForecastError::InvalidProbability(p));
        }
        Ok(Self(p.clamp(PROB_EPS, 1.0 - PROB_EPS)))
    }

    #[must_use]
    pub fn clamp_lossy(p: f64) -> Self {
        let safe = if p.is_finite() { p } else { 0.5 };
        Self(safe.clamp(PROB_EPS, 1.0 - PROB_EPS))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn logit(self) -> f64 {
        logit(self.0)
    }
}

#[must_use]
pub fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

#[must_use]
pub fn logit(p: f64) -> f64 {
    let q = p.clamp(PROB_EPS, 1.0 - PROB_EPS);
    (q / (1.0 - q)).ln()
}

#[must_use]
pub fn kl_bern(q: f64, p: f64) -> f64 {
    let q = q.clamp(PROB_EPS, 1.0 - PROB_EPS);
    let p = p.clamp(PROB_EPS, 1.0 - PROB_EPS);
    q * (q / p).ln() + (1.0 - q) * ((1.0 - q) / (1.0 - p)).ln()
}

#[must_use]
pub fn entropy_bern(p: f64) -> f64 {
    let p = p.clamp(PROB_EPS, 1.0 - PROB_EPS);
    -p * p.ln() - (1.0 - p) * (1.0 - p).ln()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizonGeometry {
    PointInTime,
    RollingWindow,
    CumulativeStoppingTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatusAtCutoff {
    Unresolved,
    Resolved,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    InitialIssuance,
    StreamLikelihoodTick,
    DeepBeliefFold,
    DataPackVintageArrival,
    CalibrationReplay,
    ManualAnalystIssue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutcomeDistribution {
    Binary { p: ProbClamped, n_eff: f64 },
    Categorical { probs: Vec<f64>, dirichlet_n: f64 },
    Cdf201 { grid: Vec<f64>, n_eff: f64 },
}

impl OutcomeDistribution {
    pub fn validate(&self) -> Result<(), ForecastError> {
        match self {
            Self::Binary { n_eff, .. } => {
                if !n_eff.is_finite() || *n_eff < 0.0 {
                    return Err(ForecastError::InvalidProbability(*n_eff));
                }
                Ok(())
            }
            Self::Categorical { probs, dirichlet_n } => {
                if probs.len() < 2 || !dirichlet_n.is_finite() || *dirichlet_n < 0.0 {
                    return Err(ForecastError::InvalidCategorical("need >= 2 outcomes and finite n_eff"));
                }
                if probs.iter().any(|p| !p.is_finite() || *p < 0.0 || *p > 1.0) {
                    return Err(ForecastError::InvalidCategorical("element out of [0, 1]"));
                }
                let sum: f64 = probs.iter().sum();
                if (sum - 1.0).abs() > 1e-6 {
                    return Err(ForecastError::InvalidCategorical("simplex sum != 1"));
                }
                Ok(())
            }
            Self::Cdf201 { grid, n_eff } => {
                if grid.len() < 2 || !n_eff.is_finite() || *n_eff < 0.0 {
                    return Err(ForecastError::InvalidCdf("grid must have >= 2 points"));
                }
                for win in grid.windows(2) {
                    if !win[0].is_finite() || !win[1].is_finite() || win[1] < win[0] {
                        return Err(ForecastError::InvalidCdf("cdf must be finite and non-decreasing"));
                    }
                }
                Ok(())
            }
        }
    }
}

/// Exact 5-component logit attribution waterfall (`§11.4` & `§16.3`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AttributionWaterfall {
    pub decay: f64,
    pub stream: f64,
    pub fold: f64,
    pub red_team: f64,
    pub kalman: f64,
}

impl AttributionWaterfall {
    #[must_use]
    pub fn total_delta_logit(&self) -> f64 {
        self.decay + self.stream + self.fold + self.red_team + self.kalman
    }

    pub fn apply_and_verify(&self, p_prev: ProbClamped) -> Result<ProbClamped, ForecastError> {
        let th_prev = p_prev.logit();
        let th_next = th_prev + self.total_delta_logit();
        let p_next = ProbClamped::clamp_lossy(sigmoid(th_next));
        let residual = ((p_next.logit() - th_prev) - self.total_delta_logit()).abs();
        if th_next.abs() < 12.0 && residual > 1e-10 {
            return Err(ForecastError::WaterfallResidual(residual));
        }
        Ok(p_next)
    }
}

/// Canonical 12-field immutable `ForecastRevision` (`KA-00`, `§7.1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForecastRevision {
    pub forecast_id: String,
    pub revision_id: String,
    pub seq_no: u32,
    pub parent_revision_id: Option<String>,
    pub question_id: String,
    pub as_of_utc: String,
    pub p_indep: ProbClamped,
    pub p_market: Option<ProbClamped>,
    pub probability_distribution: OutcomeDistribution,
    pub evidence_handles: Vec<VerifiedEvidenceRef>,
    pub model_version: String,
    pub calibration_version: String,
    pub rationale_handle: VerifiedEvidenceRef,
    pub resolution_status: ResolutionStatusAtCutoff,
}

/// Unsealed builder ensuring `seq_no == 0 <=> parent_revision_id.is_none()` and deterministic `BLAKE3` ID.
#[derive(Debug, Clone)]
pub struct UnsealedForecastRevision {
    pub forecast_id: String,
    pub seq_no: u32,
    pub parent_revision_id: Option<String>,
    pub question_id: String,
    pub as_of_utc: String,
    pub p_indep: ProbClamped,
    pub p_market: Option<ProbClamped>,
    pub probability_distribution: OutcomeDistribution,
    pub evidence_handles: Vec<VerifiedEvidenceRef>,
    pub model_version: String,
    pub calibration_version: String,
    pub rationale_handle: VerifiedEvidenceRef,
    pub resolution_status: ResolutionStatusAtCutoff,
}

impl UnsealedForecastRevision {
    pub fn seal(mut self) -> Result<ForecastRevision, ForecastError> {
        if self.seq_no == 0 && self.parent_revision_id.is_some() {
            return Err(ForecastError::LifecycleInvariant(
                "InitialForecast (seq_no=0) must have parent_revision_id = None",
            ));
        }
        if self.seq_no > 0 && self.parent_revision_id.is_none() {
            return Err(ForecastError::LifecycleInvariant(
                "SubsequentRevision (seq_no>0) must have parent_revision_id = Some(...)",
            ));
        }
        self.probability_distribution.validate()?;
        self.evidence_handles.sort();
        self.evidence_handles.dedup();

        let digest_input = serde_json::json!({
            "forecast_id": &self.forecast_id,
            "seq_no": self.seq_no,
            "parent_revision_id": &self.parent_revision_id,
            "question_id": &self.question_id,
            "as_of_utc": &self.as_of_utc,
            "p_indep": (self.p_indep.get() * 1e9).round() / 1e9,
            "p_market": self.p_market.map(|p| (p.get() * 1e9).round() / 1e9),
            "probability_distribution": &self.probability_distribution,
            "evidence_handles": &self.evidence_handles,
            "model_version": &self.model_version,
            "calibration_version": &self.calibration_version,
            "rationale_handle": &self.rationale_handle,
            "resolution_status": &self.resolution_status,
        });
        let canonical = digest_input.to_string();
        let hash = blake3::hash(canonical.as_bytes());
        let revision_id = format!("rev_{}", hash.to_hex());

        Ok(ForecastRevision {
            forecast_id: self.forecast_id,
            revision_id,
            seq_no: self.seq_no,
            parent_revision_id: self.parent_revision_id,
            question_id: self.question_id,
            as_of_utc: self.as_of_utc,
            p_indep: self.p_indep,
            p_market: self.p_market,
            probability_distribution: self.probability_distribution,
            evidence_handles: self.evidence_handles,
            model_version: self.model_version,
            calibration_version: self.calibration_version,
            rationale_handle: self.rationale_handle,
            resolution_status: self.resolution_status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_is_deterministic_and_enforces_lifecycle() {
        let ev1 = VerifiedEvidenceRef::parse("[ev:01234567#ov]").expect("valid handle");
        let rat = VerifiedEvidenceRef::parse("[ev:89abcdef#rat0]").expect("valid handle");
        let base = UnsealedForecastRevision {
            forecast_id: "fc_baltic".into(),
            seq_no: 0,
            parent_revision_id: None,
            question_id: "q_baltic".into(),
            as_of_utc: "2026-11-01T10:00:00Z".into(),
            p_indep: ProbClamped::new(0.18).expect("prob"),
            p_market: Some(ProbClamped::new(0.15).expect("prob")),
            probability_distribution: OutcomeDistribution::Binary {
                p: ProbClamped::new(0.17).expect("prob"),
                n_eff: 8.0,
            },
            evidence_handles: vec![ev1],
            model_version: "gemini-4-argon@2026-05-20".into(),
            calibration_version: "cal-v1".into(),
            rationale_handle: rat,
            resolution_status: ResolutionStatusAtCutoff::Unresolved,
        };
        let r_a = base.clone().seal().expect("seal a");
        let r_b = base.seal().expect("seal b");
        assert_eq!(r_a.revision_id, r_b.revision_id);
        assert!(r_a.revision_id.starts_with("rev_"));
        assert_eq!(r_a.revision_id.len(), 68);
    }

    #[test]
    fn waterfall_exactness() {
        let wf = AttributionWaterfall {
            decay: -0.04,
            stream: 0.18,
            fold: 0.31,
            red_team: -0.07,
            kalman: -0.11,
        };
        let p_next = wf
            .apply_and_verify(ProbClamped::new(0.134).expect("prob"))
            .expect("exact waterfall");
        assert!(p_next.get() > 0.16 && p_next.get() < 0.18);
    }
}
