//! `fcast-epistemic` (DAG Layer L3 — `SUB-03` Notary & `SUB-04` Loom).
//!
//! Hosts bi-temporal verification, staged-reveal `6D+2G` rationale scoring, causal KG,
//! and the Two-Speed Dynamic Belief Engine (`M1`–`M3`).

use serde::{Deserialize, Serialize};

/// Outcome of the gated pre-update Red-Team audit (`templates/red_team_audit.tmpl`, `§11.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedTeamAction {
    Approve,
    Dampen,
    BlockContractMisreadG1,
}
