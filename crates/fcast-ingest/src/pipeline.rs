//! Axiomatic Split 1 (`FR3a` vs `FR3b`): strictly separates pre-router Contract & Mark Intake
//! (`IntakeSnapshot`) from post-router Plan-Driven Evidence Acquisition (`EvidenceAcquisitionPlan`
//! -> `EvidenceBundle`), eliminating the `Harvester <-> RepRouter` 2-cycle.

use crate::domain_gate::DomainDecision;
use crate::microstructure::BookMarkPrice;
use fcast_core::blinding::VerifiedEvidenceRef;
use serde::{Deserialize, Serialize};

/// `FR3a` Output (runs strictly BEFORE `RepRouter`): lightweight contract metadata,
/// domain decision, and orderbook liquidity mark — zero metered research I/O.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntakeSnapshot {
    pub question_id: String,
    pub platform_id: String,
    pub question_type: String,
    pub domain_decision: DomainDecision,
    pub book_mark: Option<BookMarkPrice>,
    /// Optional complexity prior in `[0, 100]` estimated from contract structure alone.
    pub complexity_score: f64,
    /// Prize weight multiplier (e.g., FutureEval > Mantic > MiniBench > Market Pulse).
    pub prize_weight: f64,
}

impl IntakeSnapshot {
    /// Returns top-of-book liquidity credibility `phi_liq` (`0.0` if unpriced or thin).
    #[must_use]
    pub fn phi_liq(&self) -> f64 {
        self.book_mark.as_ref().map_or(0.0, |m| m.phi_liq)
    }
}

/// `FR3b` Input (emitted strictly BY `RepRouter` Phase B): explicit evidence acquisition directives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceAcquisitionPlan {
    pub question_id: String,
    pub as_of_utc: String,
    pub fetch_raw_data: bool,
    pub attach_seasonal_note: bool,
    pub dual_independent_reports: bool,
    pub zero_key_fallback: bool,
}

/// `FR3b` Output (runs strictly AFTER `RepRouter`): vaulted evidence handles ready for `Loom`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub question_id: String,
    pub as_of_utc: String,
    pub evidence_handles: Vec<VerifiedEvidenceRef>,
    pub seasonal_note_attached: bool,
    pub raw_data_attached: bool,
    pub report_count: u8,
}

/// Deterministic `FR3b` plan executor over pre-vaulted candidate handles (enforcing plan flags).
#[must_use]
pub fn assemble_evidence_bundle(
    plan: &EvidenceAcquisitionPlan,
    vaulted_handles: Vec<VerifiedEvidenceRef>,
) -> EvidenceBundle {
    let mut handles = vaulted_handles;
    handles.sort();
    handles.dedup();
    let report_count = if plan.dual_independent_reports { 2 } else { 1 };
    EvidenceBundle {
        question_id: plan.question_id.clone(),
        as_of_utc: plan.as_of_utc.clone(),
        evidence_handles: handles,
        seasonal_note_attached: plan.attach_seasonal_note,
        raw_data_attached: plan.fetch_raw_data,
        report_count,
    }
}
