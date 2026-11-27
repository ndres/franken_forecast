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
    #[error("revision_id mismatch: expected canonical {expected}, got {actual}")]
    RevisionIdMismatch { expected: String, actual: String },
}

/// Strictly clamped probability in `[1e-6, 1 - 1e-6]`, validated on both construction and deserialization.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct ProbClamped(f64);

impl TryFrom<f64> for ProbClamped {
    type Error = ForecastError;
    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProbClamped> for f64 {
    fn from(value: ProbClamped) -> Self {
        value.0
    }
}

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
                    return Err(ForecastError::InvalidCategorical(
                        "need >= 2 outcomes and finite n_eff",
                    ));
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
                // Shared across continuous 201-point CDFs, discrete (inbound_outcome_count + 1) grids,
                // and Mantic N<=31 CDFs; enforces 2..=201 length, [0.0, 1.0] bounds, and monotonicity.
                if grid.len() < 2 || grid.len() > 201 || !n_eff.is_finite() || *n_eff < 0.0 {
                    return Err(ForecastError::InvalidCdf(
                        "grid must have 2..=201 points and finite non-negative n_eff",
                    ));
                }
                if grid
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                {
                    return Err(ForecastError::InvalidCdf("cdf values must be in [0, 1]"));
                }
                for win in grid.windows(2) {
                    if win[1] < win[0] {
                        return Err(ForecastError::InvalidCdf("cdf must be non-decreasing"));
                    }
                }
                Ok(())
            }
        }
    }

    /// Canonical 1e-9-rounded JSON projection used inside `ForecastRevision` BLAKE3 hashing
    /// so cross-platform float ULP drift never alters `revision_id`.
    #[must_use]
    pub fn canonical_digest_json(&self) -> serde_json::Value {
        let round9 = |x: f64| -> f64 { (x * 1e9).round() / 1e9 };
        match self {
            Self::Binary { p, n_eff } => serde_json::json!({
                "kind": "binary",
                "p": round9(p.get()),
                "n_eff": round9(*n_eff),
            }),
            Self::Categorical { probs, dirichlet_n } => serde_json::json!({
                "kind": "categorical",
                "probs": probs.iter().map(|&v| round9(v)).collect::<Vec<_>>(),
                "dirichlet_n": round9(*dirichlet_n),
            }),
            Self::Cdf201 { grid, n_eff } => serde_json::json!({
                "kind": "cdf201",
                "grid": grid.iter().map(|&v| round9(v)).collect::<Vec<_>>(),
                "n_eff": round9(*n_eff),
            }),
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
        for component in [
            self.decay,
            self.stream,
            self.fold,
            self.red_team,
            self.kalman,
        ] {
            if !component.is_finite() {
                return Err(ForecastError::InvalidProbability(component));
            }
        }
        let delta = self.total_delta_logit();
        if !delta.is_finite() {
            return Err(ForecastError::InvalidProbability(delta));
        }
        let th_prev = p_prev.logit();
        let th_next = th_prev + delta;
        let p_next = ProbClamped::clamp_lossy(sigmoid(th_next));
        let residual = ((p_next.logit() - th_prev) - delta).abs();
        if th_next.abs() < 12.0 && residual > 1e-10 {
            return Err(ForecastError::WaterfallResidual(residual));
        }
        Ok(p_next)
    }
}

/// Canonical 12-field immutable `ForecastRevision` (`KA-00`, `§7.1`).
///
/// Deserialization validates lifecycle invariants, `probability_distribution`, and exact
/// canonical BLAKE3 `revision_id` integrity via [`ForecastRevision::verify_integrity`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ForecastRevisionDto")]
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

#[derive(Deserialize)]
struct ForecastRevisionDto {
    forecast_id: String,
    revision_id: String,
    seq_no: u32,
    parent_revision_id: Option<String>,
    question_id: String,
    as_of_utc: String,
    p_indep: ProbClamped,
    p_market: Option<ProbClamped>,
    probability_distribution: OutcomeDistribution,
    evidence_handles: Vec<VerifiedEvidenceRef>,
    model_version: String,
    calibration_version: String,
    rationale_handle: VerifiedEvidenceRef,
    resolution_status: ResolutionStatusAtCutoff,
}

impl TryFrom<ForecastRevisionDto> for ForecastRevision {
    type Error = ForecastError;
    fn try_from(dto: ForecastRevisionDto) -> Result<Self, Self::Error> {
        let rev = Self {
            forecast_id: dto.forecast_id,
            revision_id: dto.revision_id,
            seq_no: dto.seq_no,
            parent_revision_id: dto.parent_revision_id,
            question_id: dto.question_id,
            as_of_utc: dto.as_of_utc,
            p_indep: dto.p_indep,
            p_market: dto.p_market,
            probability_distribution: dto.probability_distribution,
            evidence_handles: dto.evidence_handles,
            model_version: dto.model_version,
            calibration_version: dto.calibration_version,
            rationale_handle: dto.rationale_handle,
            resolution_status: dto.resolution_status,
        };
        rev.verify_integrity()?;
        Ok(rev)
    }
}

impl ForecastRevision {
    /// Recomputes the canonical BLAKE3 `revision_id` from normalized fields (all floats rounded to `1e-9`).
    #[must_use]
    pub fn compute_canonical_revision_id(&self) -> String {
        let mut sorted_handles = self.evidence_handles.clone();
        sorted_handles.sort();
        sorted_handles.dedup();
        compute_revision_id_parts(
            &self.forecast_id,
            self.seq_no,
            self.parent_revision_id.as_deref(),
            &self.question_id,
            &self.as_of_utc,
            self.p_indep,
            self.p_market,
            &self.probability_distribution,
            &sorted_handles,
            &self.model_version,
            &self.calibration_version,
            &self.rationale_handle,
            self.resolution_status,
        )
    }

    /// Re-verifies lifecycle invariants, distribution validity, and exact BLAKE3 `revision_id` match.
    pub fn verify_integrity(&self) -> Result<(), ForecastError> {
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
        let expected = self.compute_canonical_revision_id();
        if self.revision_id != expected {
            return Err(ForecastError::RevisionIdMismatch {
                expected,
                actual: self.revision_id.clone(),
            });
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_revision_id_parts(
    forecast_id: &str,
    seq_no: u32,
    parent_revision_id: Option<&str>,
    question_id: &str,
    as_of_utc: &str,
    p_indep: ProbClamped,
    p_market: Option<ProbClamped>,
    probability_distribution: &OutcomeDistribution,
    evidence_handles: &[VerifiedEvidenceRef],
    model_version: &str,
    calibration_version: &str,
    rationale_handle: &VerifiedEvidenceRef,
    resolution_status: ResolutionStatusAtCutoff,
) -> String {
    let digest_input = serde_json::json!({
        "forecast_id": forecast_id,
        "seq_no": seq_no,
        "parent_revision_id": parent_revision_id,
        "question_id": question_id,
        "as_of_utc": as_of_utc,
        "p_indep": (p_indep.get() * 1e9).round() / 1e9,
        "p_market": p_market.map(|p| (p.get() * 1e9).round() / 1e9),
        "probability_distribution": probability_distribution.canonical_digest_json(),
        "evidence_handles": evidence_handles,
        "model_version": model_version,
        "calibration_version": calibration_version,
        "rationale_handle": rationale_handle,
        "resolution_status": resolution_status,
    });
    let canonical = digest_input.to_string();
    let hash = blake3::hash(canonical.as_bytes());
    format!("rev_{}", hash.to_hex())
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

        let revision_id = compute_revision_id_parts(
            &self.forecast_id,
            self.seq_no,
            self.parent_revision_id.as_deref(),
            &self.question_id,
            &self.as_of_utc,
            self.p_indep,
            self.p_market,
            &self.probability_distribution,
            &self.evidence_handles,
            &self.model_version,
            &self.calibration_version,
            &self.rationale_handle,
            self.resolution_status,
        );

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

        let nan_wf = AttributionWaterfall {
            decay: f64::NAN,
            stream: 0.0,
            fold: 0.0,
            red_team: 0.0,
            kalman: 0.0,
        };
        assert!(
            nan_wf
                .apply_and_verify(ProbClamped::new(0.50).expect("prob"))
                .is_err()
        );
    }

    #[test]
    fn serde_enforces_prob_clamped_cdf_bounds_and_revision_integrity() {
        // 1. ProbClamped rejects out-of-range values during serde deserialization
        assert!(serde_json::from_str::<ProbClamped>("0.0").is_err());
        assert!(serde_json::from_str::<ProbClamped>("1.0").is_err());
        assert!(serde_json::from_str::<ProbClamped>("-0.1").is_err());
        assert!(serde_json::from_str::<ProbClamped>("2.5").is_err());
        let ok_p: ProbClamped = serde_json::from_str("0.42").expect("valid prob");
        assert!((ok_p.get() - 0.42).abs() < 1e-12);

        // 2. Cdf201 enforces [0, 1] bounds and 2..=201 length
        assert!(
            OutcomeDistribution::Cdf201 {
                grid: vec![-0.01, 0.5, 1.0],
                n_eff: 5.0,
            }
            .validate()
            .is_err()
        );
        assert!(
            OutcomeDistribution::Cdf201 {
                grid: vec![0.0, 0.5, 1.05],
                n_eff: 5.0,
            }
            .validate()
            .is_err()
        );
        assert!(
            OutcomeDistribution::Cdf201 {
                grid: vec![0.5],
                n_eff: 5.0,
            }
            .validate()
            .is_err()
        );
        assert!(
            OutcomeDistribution::Cdf201 {
                grid: vec![0.1, 0.4, 0.8, 0.99],
                n_eff: 5.0,
            }
            .validate()
            .is_ok()
        );

        // 3. ForecastRevision rejects tampered revision_id on serde deserialize and rounds distribution floats
        let rat = VerifiedEvidenceRef::parse("[ev:89abcdef#rat0]").expect("rat");
        let r1 = UnsealedForecastRevision {
            forecast_id: "fc_hash".into(),
            seq_no: 0,
            parent_revision_id: None,
            question_id: "q_hash".into(),
            as_of_utc: "2026-11-27T10:00:00Z".into(),
            p_indep: ProbClamped::new(0.35).expect("p"),
            p_market: None,
            probability_distribution: OutcomeDistribution::Binary {
                p: ProbClamped::new(0.35).expect("p"),
                n_eff: 10.0,
            },
            evidence_handles: vec![],
            model_version: "m@v1".into(),
            calibration_version: "cal-v1".into(),
            rationale_handle: rat.clone(),
            resolution_status: ResolutionStatusAtCutoff::Unresolved,
        }
        .seal()
        .expect("seal r1");
        let r2 = UnsealedForecastRevision {
            forecast_id: "fc_hash".into(),
            seq_no: 0,
            parent_revision_id: None,
            question_id: "q_hash".into(),
            as_of_utc: "2026-11-27T10:00:00Z".into(),
            p_indep: ProbClamped::new(0.35 + 1e-12).expect("p"),
            p_market: None,
            probability_distribution: OutcomeDistribution::Binary {
                p: ProbClamped::new(0.35 + 1e-12).expect("p"),
                n_eff: 10.0 + 1e-12,
            },
            evidence_handles: vec![],
            model_version: "m@v1".into(),
            calibration_version: "cal-v1".into(),
            rationale_handle: rat,
            resolution_status: ResolutionStatusAtCutoff::Unresolved,
        }
        .seal()
        .expect("seal r2");
        assert_eq!(r1.revision_id, r2.revision_id);

        let mut val = serde_json::to_value(&r1).expect("to_value");
        let ok_roundtrip: ForecastRevision =
            serde_json::from_value(val.clone()).expect("valid roundtrip");
        assert_eq!(ok_roundtrip.revision_id, r1.revision_id);

        val["p_indep"] = serde_json::json!(0.91);
        assert!(serde_json::from_value::<ForecastRevision>(val).is_err());
    }
}
