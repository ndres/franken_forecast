//! Staged-reveal `6D+2G` forecaster rationale scoring (`§10`, `KA-04`, `GATE-16`, `GATE-17`, `GATE-18`).
//!
//! Enforces:
//! - `INV-05`: `TurnARequest` physically omits `forecaster_forecast` and `crowd_prior`.
//! - `INV-06`: Zero LLM arithmetic — Rust computes `RankTier`, `effective_score`, and deterministic tie-breaks.

use std::cmp::Reverse;

use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::forecast::OutcomeDistribution;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::verify::{TemplateCatalog, TemplateId};

/// Errors emitted by the staged-reveal rationale scorer and ranker.
#[derive(Debug, Error, PartialEq)]
pub enum RationaleError {
    #[error("unscoreable=true requires all 6 dimensions to be None and g1=false, g2=false")]
    InvalidUnscoreablePayload,
    #[error("scoreable rationale missing required dimension {0}")]
    MissingDimension(&'static str),
    #[error("dimension {dim} out of allowed 0..=4 range: {value}")]
    DimensionOutOfRange { dim: &'static str, value: u8 },
    #[error("gate {gate} set without required justification note (>= 10 chars)")]
    MissingGateJustification { gate: &'static str },
    #[error("Turn B mutated locked Turn A dimension {dim}: expected {expected:?}, got {actual:?}")]
    TurnALockTampered {
        dim: &'static str,
        expected: Option<u8>,
        actual: Option<u8>,
    },
    #[error("template {0:?} missing from TemplateCatalog")]
    MissingTemplate(TemplateId),
    #[error(
        "spearman rank correlation requires equal-length vectors of length >= 2 (got {0} vs {1})"
    )]
    InvalidSpearmanInput(usize, usize),
    #[error("spearman rank correlation undefined for zero-variance rank vector")]
    ZeroVarianceRanks,
}

// ---------------------------------------------------------------------------
// 3-Tier crowd_prior Cascade (§10)
// ---------------------------------------------------------------------------

/// Source tier that supplied `crowd_prior` (`§10`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrowdPriorSource {
    VenueLive,
    ForecastBenchMedian,
    ClusterConsensus,
}

/// Optional ForecastBench published median row subject to `available_utc <= freeze_date`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForecastBenchMedianRow {
    pub freeze_crowd_median: Option<f64>,
    pub sf_median: Option<f64>,
    pub public_median: Option<f64>,
    pub available_utc: String,
}

/// Resolves `crowd_prior` via the 3-tier cascade (`§10`):
/// 1. Unmasked live venue `p_micro` passing `L_tob_min`,
/// 2. `forecastbench-datasets` published median (`available_utc <= as_of_utc`),
/// 3. Credibility-weighted `EquivalenceCluster` median.
///
/// Returns `None` unconditionally when `blind_pre_seal == true` (`INV-04`/`INV-05`).
#[must_use]
pub fn resolve_crowd_prior(
    venue_tob_prob: Option<f64>,
    venue_thin_or_masked: bool,
    fb_median_row: Option<&ForecastBenchMedianRow>,
    cluster_median: Option<f64>,
    as_of_utc: &str,
    blind_pre_seal: bool,
) -> Option<(CrowdPriorSource, f64)> {
    if blind_pre_seal {
        return None;
    }
    if !venue_thin_or_masked && let Some(p) = venue_tob_prob {
        return Some((CrowdPriorSource::VenueLive, p));
    }
    if let Some(row) = fb_median_row
        && fcast_core::time::is_utc_le(&row.available_utc, as_of_utc)
        && let Some(val) = row
            .freeze_crowd_median
            .or(row.sf_median)
            .or(row.public_median)
    {
        return Some((CrowdPriorSource::ForecastBenchMedian, val));
    }
    cluster_median.map(|p| (CrowdPriorSource::ClusterConsensus, p))
}

// ---------------------------------------------------------------------------
// Mechanical 2-Turn Staged Reveal Structs (INV-05, GATE-16)
// ---------------------------------------------------------------------------

/// Turn A price-blind request (`INV-05`, `GATE-16`).
///
/// Physically omits `forecaster_forecast` and `crowd_prior` at the Rust type level so
/// serialized wire JSON can never leak price context into Turn A.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnARequest {
    pub question_id: String,
    pub as_of_utc: String,
    pub contract_sha256: String,
    pub resolution_criteria: String,
    pub background_as_of_utc: String,
    pub rationale_handle: VerifiedEvidenceRef,
    pub rationale_text: String,
}

impl TurnARequest {
    /// Serializes the Turn A wire request to JSON.
    pub fn to_wire_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Renders `templates/rationale_staged_turn_a.tmpl` with Turn A fields only.
    pub fn render_prompt(&self, catalog: &TemplateCatalog) -> Result<String, RationaleError> {
        let tmpl = catalog.get(TemplateId::RationaleStagedTurnA).ok_or(
            RationaleError::MissingTemplate(TemplateId::RationaleStagedTurnA),
        )?;
        Ok(tmpl
            .body
            .replace("{question_id}", &self.question_id)
            .replace("{as_of_utc}", &self.as_of_utc)
            .replace("{contract_sha256}", &self.contract_sha256)
            .replace("{resolution_criteria}", &self.resolution_criteria)
            .replace("{background_as_of_utc}", &self.background_as_of_utc)
            .replace("{rationale_handle}", self.rationale_handle.as_str())
            .replace("{rationale_text}", &self.rationale_text))
    }
}

/// Locked integer scores returned by Turn A (`ct, ei, ov, me, ej ∈ 0..=4` or `unscoreable = true`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedTurnAScores {
    pub question_id: String,
    pub rationale_handle: VerifiedEvidenceRef,
    pub as_of_utc: String,
    pub ct: Option<u8>,
    pub ei: Option<u8>,
    pub ov: Option<u8>,
    pub me: Option<u8>,
    pub ej: Option<u8>,
    pub unscoreable: bool,
}

impl LockedTurnAScores {
    /// Validates Turn A score invariants (`0..=4` when scoreable; all `None` when `unscoreable`).
    pub fn validate(&self) -> Result<(), RationaleError> {
        let dims = [
            ("ct", self.ct),
            ("ei", self.ei),
            ("ov", self.ov),
            ("me", self.me),
            ("ej", self.ej),
        ];
        if self.unscoreable {
            if dims.iter().any(|(_, v)| v.is_some()) {
                return Err(RationaleError::InvalidUnscoreablePayload);
            }
            return Ok(());
        }
        for (name, val) in dims {
            let v = val.ok_or(RationaleError::MissingDimension(name))?;
            if v > 4 {
                return Err(RationaleError::DimensionOutOfRange {
                    dim: name,
                    value: v,
                });
            }
        }
        Ok(())
    }
}

/// Turn B reveal request (`INV-05`, `GATE-16`).
///
/// Echoes locked Turn A integers and reveals `(forecaster_forecast, crowd_prior)` via
/// `previous_interaction_id` to score `pr`, `g1`, `g2`, and `note`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnBRequest {
    pub previous_interaction_id: String,
    pub locked_turn_a: LockedTurnAScores,
    pub as_of_utc: String,
    pub forecaster_forecast: OutcomeDistribution,
    pub crowd_prior: Option<f64>,
    pub crowd_prior_source: Option<CrowdPriorSource>,
}

impl TurnBRequest {
    /// Constructs a validated `TurnBRequest` after verifying `locked_turn_a`.
    pub fn new(
        previous_interaction_id: String,
        locked_turn_a: LockedTurnAScores,
        as_of_utc: String,
        forecaster_forecast: OutcomeDistribution,
        crowd_prior: Option<(CrowdPriorSource, f64)>,
    ) -> Result<Self, RationaleError> {
        locked_turn_a.validate()?;
        let (crowd_prior_source, crowd_prior_val) = match crowd_prior {
            Some((src, val)) => (Some(src), Some(val)),
            None => (None, None),
        };
        Ok(Self {
            previous_interaction_id,
            locked_turn_a,
            as_of_utc,
            forecaster_forecast,
            crowd_prior: crowd_prior_val,
            crowd_prior_source,
        })
    }

    /// Serializes the Turn B wire request to JSON.
    pub fn to_wire_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Verifies that Turn B echoed all locked Turn A integers unchanged and constructs a
    /// `RationaleRecord` ready for deterministic Rust scoring.
    pub fn finalize_record(
        &self,
        id: String,
        created_at_utc: String,
        turn_b: TurnBResponse,
    ) -> Result<RationaleRecord, RationaleError> {
        let checks = [
            ("ct", self.locked_turn_a.ct, turn_b.ct),
            ("ei", self.locked_turn_a.ei, turn_b.ei),
            ("ov", self.locked_turn_a.ov, turn_b.ov),
            ("me", self.locked_turn_a.me, turn_b.me),
            ("ej", self.locked_turn_a.ej, turn_b.ej),
        ];
        for (dim, expected, actual) in checks {
            if expected != actual {
                return Err(RationaleError::TurnALockTampered {
                    dim,
                    expected,
                    actual,
                });
            }
        }
        Ok(RationaleRecord {
            id,
            ct: turn_b.ct,
            ei: turn_b.ei,
            ov: turn_b.ov,
            me: turn_b.me,
            ej: turn_b.ej,
            pr: turn_b.pr,
            g1: turn_b.g1,
            g2: turn_b.g2,
            unscoreable: turn_b.unscoreable,
            note: turn_b.note,
            created_at_utc,
        })
    }
}

/// Turn B response echoing locked Turn A scores and adding `(pr, g1, g2, note)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBResponse {
    pub ct: Option<u8>,
    pub ei: Option<u8>,
    pub ov: Option<u8>,
    pub me: Option<u8>,
    pub ej: Option<u8>,
    pub pr: Option<u8>,
    pub g1: bool,
    pub g2: bool,
    pub unscoreable: bool,
    pub note: String,
}

// ---------------------------------------------------------------------------
// Zero-LLM-Arithmetic 6D+2G Scoring & Deterministic Ranking (INV-06, GATE-17)
// ---------------------------------------------------------------------------

/// Input record for deterministic `6D+2G` rationale scoring (`§10`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RationaleRecord {
    pub id: String,
    pub ct: Option<u8>,
    pub ei: Option<u8>,
    pub ov: Option<u8>,
    pub me: Option<u8>,
    pub ej: Option<u8>,
    pub pr: Option<u8>,
    pub g1: bool,
    pub g2: bool,
    pub unscoreable: bool,
    pub note: String,
    /// ISO-8601 / RFC-3339 UTC timestamp (lexicographically sortable; earlier wins ties).
    pub created_at_utc: String,
}

/// Strict rank tier ordering (`DisqualifiedG2 < CappedG1 < Clean`, `§10`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankTier {
    /// `g2 == true`: post-cutoff leak or fabrication (`effective_score = 0`).
    DisqualifiedG2 = 0,
    /// `g1 == true && !g2`: load-bearing contract misread (`effective_score = min(sum, 8)`).
    CappedG1 = 1,
    /// Clean rationale (`effective_score = ct + ei + ov + me + ej + pr in 0..=24`).
    Clean = 2,
}

/// Deterministically scored rationale (`INV-06`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoredRationale {
    pub id: String,
    pub tier: RankTier,
    pub raw_sum: u8,
    pub effective_score: u8,
    pub ct: u8,
    pub ei: u8,
    pub ov: u8,
    pub me: u8,
    pub ej: u8,
    pub pr: u8,
    pub created_at_utc: String,
}

impl ScoredRationale {
    /// Exact deterministic ranking key (`§10`):
    /// `(tier, eff, min(ct, me), ei, ov, ej, pr, Reverse(created_at_utc))`.
    #[must_use]
    pub fn rank_key(&self) -> (RankTier, u8, u8, u8, u8, u8, u8, Reverse<&str>) {
        (
            self.tier,
            self.effective_score,
            self.ct.min(self.me),
            self.ei,
            self.ov,
            self.ej,
            self.pr,
            Reverse(self.created_at_utc.as_str()),
        )
    }
}

/// Validates and scores a single `RationaleRecord` with zero LLM arithmetic (`INV-06`).
///
/// Returns `Ok(None)` when `unscoreable == true` (excluded from leaderboard).
pub fn score_rationale(r: &RationaleRecord) -> Result<Option<ScoredRationale>, RationaleError> {
    let dims = [
        ("ct", r.ct),
        ("ei", r.ei),
        ("ov", r.ov),
        ("me", r.me),
        ("ej", r.ej),
        ("pr", r.pr),
    ];

    if r.unscoreable {
        if dims.iter().any(|(_, v)| v.is_some()) || r.g1 || r.g2 {
            return Err(RationaleError::InvalidUnscoreablePayload);
        }
        return Ok(None);
    }

    let mut vals = [0_u8; 6];
    for (idx, (name, opt)) in dims.iter().enumerate() {
        let v = opt.ok_or(RationaleError::MissingDimension(name))?;
        if v > 4 {
            return Err(RationaleError::DimensionOutOfRange {
                dim: name,
                value: v,
            });
        }
        vals[idx] = v;
    }

    if (r.g1 || r.g2) && r.note.trim().chars().count() < 10 {
        return Err(RationaleError::MissingGateJustification {
            gate: if r.g2 { "g2" } else { "g1" },
        });
    }

    let [ct, ei, ov, me, ej, pr] = vals;
    let raw_sum = ct + ei + ov + me + ej + pr;
    let (tier, effective_score) = if r.g2 {
        (RankTier::DisqualifiedG2, 0)
    } else if r.g1 {
        (RankTier::CappedG1, raw_sum.min(8))
    } else {
        (RankTier::Clean, raw_sum)
    };

    Ok(Some(ScoredRationale {
        id: r.id.clone(),
        tier,
        raw_sum,
        effective_score,
        ct,
        ei,
        ov,
        me,
        ej,
        pr,
        created_at_utc: r.created_at_utc.clone(),
    }))
}

/// Scores and deterministically ranks a slice of `RationaleRecord`s (`INV-06`, `GATE-17`).
///
/// - `unscoreable == true` records are validated and excluded.
/// - Remaining records are sorted descending by
///   `(tier, eff, min(ct, me), ei, ov, ej, pr, Reverse(created_at_utc))`
///   with `Reverse(id)` as a deterministic final tie-breaker if timestamps are identical.
pub fn rank_rationales(
    records: &[RationaleRecord],
) -> Result<Vec<ScoredRationale>, RationaleError> {
    let mut scored = Vec::with_capacity(records.len());
    for rec in records {
        if let Some(s) = score_rationale(rec)? {
            scored.push(s);
        }
    }
    scored.sort_by(|a, b| {
        b.rank_key()
            .cmp(&a.rank_key())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(scored)
}

// ---------------------------------------------------------------------------
// Spearman Rank Correlation Evaluator (GATE-18)
// ---------------------------------------------------------------------------

/// Computes exact tie-aware Spearman rank correlation `rho` in `[-1.0, 1.0]` (`GATE-18`).
pub fn spearman_rank_correlation(xs: &[f64], ys: &[f64]) -> Result<f64, RationaleError> {
    if xs.len() != ys.len() || xs.len() < 2 {
        return Err(RationaleError::InvalidSpearmanInput(xs.len(), ys.len()));
    }
    let rx = average_ranks(xs);
    let ry = average_ranks(ys);
    let n = rx.len() as f64;
    let mean_x = rx.iter().sum::<f64>() / n;
    let mean_y = ry.iter().sum::<f64>() / n;

    let mut cov = 0.0_f64;
    let mut var_x = 0.0_f64;
    let mut var_y = 0.0_f64;
    for (x, y) in rx.iter().zip(ry.iter()) {
        let dx = x - mean_x;
        let dy = y - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }
    let denom = (var_x * var_y).sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return Err(RationaleError::ZeroVarianceRanks);
    }
    Ok((cov / denom).clamp(-1.0, 1.0))
}

fn average_ranks(values: &[f64]) -> Vec<f64> {
    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by(|(_, a), (_, b)| a.total_cmp(b));

    let mut ranks = vec![0.0_f64; values.len()];
    let mut i = 0_usize;
    while i < indexed.len() {
        let mut j = i + 1;
        while j < indexed.len() && (indexed[j].1 - indexed[i].1).abs() <= 1e-12 {
            j += 1;
        }
        // 1-based average rank for tied group [i..j).
        let avg_rank = ((i + 1) + j) as f64 / 2.0;
        for &(orig_idx, _) in &indexed[i..j] {
            ranks[orig_idx] = avg_rank;
        }
        i = j;
    }
    ranks
}
