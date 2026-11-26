//! `fcast-epistemic` (DAG Layer L3 — `SUB-03` Notary & `SUB-04` Loom).
//!
//! Hosts bi-temporal verification, canonical SHA-256-locked prompt template catalog (`§9`),
//! 4-rung `ValueExtractionLadder` (`§11.2`), staged-reveal `6D+2G` rationale scoring (`§10`),
//! causal KG, and the Two-Speed Dynamic Belief Engine (`M1`–`M3`).

pub mod rationales;
pub mod verify;

use serde::{Deserialize, Serialize};

pub use rationales::{
    CrowdPriorSource, ForecastBenchMedianRow, LockedTurnAScores, RankTier, RationaleError,
    RationaleRecord, ScoredRationale, TurnARequest, TurnBRequest, TurnBResponse, rank_rationales,
    resolve_crowd_prior, score_rationale, spearman_rank_correlation,
};
pub use verify::{
    AuditEvidenceItem, BiTemporalAudit, BiTemporalAuditOutcome, ExtractedOptionMap,
    ExtractedProbability, ExtractionRung, LOCKED_TEMPLATE_SPECS, LoadedTemplate, MinuteWindow,
    NORMAL_PUBLISH_BOUNDS, PerQuestionResolution, QuestionContractSpec, THIN_PUBLISH_FLOOR,
    TemplateCatalog, TemplateId, TemplateSpec, UtcMillis, ValueExtractionLadder, VerifyError,
    apply_publish_floor, parse_strict_prob_json, parse_utc_millis, sha256_hex,
};

/// Outcome of the gated pre-update Red-Team audit (`templates/red_team_audit.tmpl`, `§11.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedTeamAction {
    Approve,
    Dampen,
    BlockContractMisreadG1,
}
