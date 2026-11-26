//! Axiomatic Split 3 (`FR6a` Pre-Loom Stateless Guard vs `FR6b` Post-Proposal Binding Audit).
//!
//! Eliminates the `Loom <-> Notary` 2-cycle:
//! - `PreLoomGuard` (`FR6a`, runs BEFORE `Loom` updates belief): validates templates + extracts
//!   probabilities via `ValueExtractionLadder` + applies `THIN_PUBLISH_FLOOR`.
//! - `PostProposalAudit` (`FR6b`, runs AFTER `Loom` emits a `DraftRevisionProposal`): enforces
//!   binding `red_team_audit.tmpl` + `load_bearing_claims` `VERIFY` before sealing.

use crate::verify::{
    ExtractedProbability, ValueExtractionLadder, VerifyError, apply_publish_floor,
};
use fcast_core::forecast::{ProbClamped, logit, sigmoid};
use serde::{Deserialize, Serialize};

/// `FR6a` Stateless extraction & publish-floor guard running strictly BEFORE `Loom`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PreLoomGuard;

impl PreLoomGuard {
    /// Extracts a numeric probability from raw LLM output through `ValueExtractionLadder`
    /// and applies `THIN_PUBLISH_FLOOR` (`[0.05, 0.95]`) when `survivors == 1`.
    pub fn extract_and_guard_prob(
        raw_llm_output: &str,
        survivors: usize,
    ) -> Result<(ProbClamped, ExtractedProbability), VerifyError> {
        let extracted = ValueExtractionLadder::extract_binary_probability::<
            fn(&str) -> Result<String, String>,
        >(raw_llm_output, None)?;
        let guarded = apply_publish_floor(extracted.probability, survivors);
        let prob = ProbClamped::clamp_lossy(guarded);
        Ok((prob, extracted))
    }
}

/// Market-blind draft emitted by `Loom` (`FR5`) for post-proposal `Notary` audit (`FR6b`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftRevisionProposal {
    pub question_id: String,
    pub p_prev: ProbClamped,
    pub p_proposed: ProbClamped,
    pub is_freeze_deadline: bool,
    pub contract_misread_g1: bool,
    pub unverified_load_bearing_claims: Vec<String>,
}

/// Binding verdict returned by `PostProposalAudit` (`FR6b`) before `ForecastRevision` sealing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BindingAuditVerdict {
    /// Audit not required or passed cleanly; seal `p_proposed`.
    Approve { final_p: ProbClamped },
    /// Material shift dampened halfway back toward `p_prev` in logit space.
    DampenTowardPrior {
        final_p: ProbClamped,
        reason: String,
    },
    /// Binding block (`G1` contract misread or unverified load-bearing claim).
    BlockPublish { reason: String },
}

/// `FR6b` Post-proposal binding Red-Team & consensus-fragility gate running strictly AFTER `Loom`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PostProposalAudit;

impl PostProposalAudit {
    /// Default material-shift logit threshold (`§11.2`).
    pub const RED_TEAM_LOGIT_THRESHOLD: f64 = 0.35;

    /// Returns `true` when `red_team_audit.tmpl` is mandatory (`|dlogit| >= 0.35`, tails, or freeze).
    #[must_use]
    pub fn requires_red_team(proposal: &DraftRevisionProposal) -> bool {
        let dlogit = (proposal.p_proposed.logit() - proposal.p_prev.logit()).abs();
        let p = proposal.p_proposed.get();
        dlogit >= Self::RED_TEAM_LOGIT_THRESHOLD
            || !(0.10..=0.90).contains(&p)
            || proposal.is_freeze_deadline
    }

    /// Evaluates a `DraftRevisionProposal` and returns a binding verdict (`Approve`/`Dampen`/`Block`).
    #[must_use]
    pub fn evaluate_binding(proposal: &DraftRevisionProposal) -> BindingAuditVerdict {
        if !Self::requires_red_team(proposal) {
            return BindingAuditVerdict::Approve {
                final_p: proposal.p_proposed,
            };
        }
        if proposal.contract_misread_g1 {
            return BindingAuditVerdict::BlockPublish {
                reason: "BINDING_G1_CONTRACT_MISREAD".into(),
            };
        }
        if !proposal.unverified_load_bearing_claims.is_empty() {
            return BindingAuditVerdict::BlockPublish {
                reason: format!(
                    "UNVERIFIED_LOAD_BEARING_CLAIMS:{}",
                    proposal.unverified_load_bearing_claims.join(",")
                ),
            };
        }
        // Extreme jump (|dlogit| >= 1.50) without G1 is dampened 50% in logit space to curb overreaction.
        let dlogit = proposal.p_proposed.logit() - proposal.p_prev.logit();
        if dlogit.abs() >= 1.50 {
            let damped =
                ProbClamped::clamp_lossy(sigmoid(logit(proposal.p_prev.get()) + 0.5 * dlogit));
            return BindingAuditVerdict::DampenTowardPrior {
                final_p: damped,
                reason: "EXTREME_SHIFT_LOGIT_DAMPED_50PCT".into(),
            };
        }
        BindingAuditVerdict::Approve {
            final_p: proposal.p_proposed,
        }
    }
}
