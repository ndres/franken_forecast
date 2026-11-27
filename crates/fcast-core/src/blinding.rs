//! Handle-only evidence references (`INV-07`) and pre-seal `MarketBlindingScrubber` (`INV-04`, `INV-05`).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlindingError {
    #[error("malformed evidence handle {0:?}: expected [ev:<hex>#<span_id>]")]
    InvalidEvidenceHandle(String),
    #[error("market blinding violation ({0}): prohibited token found in pre-seal payload")]
    MarketTokenLeak(&'static str),
}

/// Validated `[ev:<hex>#<span_id>]` pointer into `EvidenceVault` (`INV-07`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VerifiedEvidenceRef(String);

impl VerifiedEvidenceRef {
    pub fn parse(raw: &str) -> Result<Self, BlindingError> {
        let trimmed = raw.trim();
        let inner = trimmed
            .strip_prefix("[ev:")
            .and_then(|s| s.strip_suffix(']'))
            .ok_or_else(|| BlindingError::InvalidEvidenceHandle(raw.to_owned()))?;
        let (hex_part, span_part) = inner
            .split_once('#')
            .ok_or_else(|| BlindingError::InvalidEvidenceHandle(raw.to_owned()))?;
        if hex_part.len() < 2
            || hex_part.len() > 64
            || !hex_part.bytes().all(|b| b.is_ascii_hexdigit())
            || span_part.is_empty()
            || !span_part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        {
            return Err(BlindingError::InvalidEvidenceHandle(raw.to_owned()));
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for VerifiedEvidenceRef {
    type Error = BlindingError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<VerifiedEvidenceRef> for String {
    fn from(value: VerifiedEvidenceRef) -> Self {
        value.0
    }
}

/// Enforces `INV-04` & `INV-05`: strips or rejects market-price & crowd-prior tokens prior to `p_indep` seal.
#[derive(Debug, Default, Clone, Copy)]
pub struct MarketBlindingScrubber;

/// Whole-string withholding marker preventing attacker splicing around partial redactions
/// (`Thesis` `private_source_screen.py` pattern).
pub const WITHHELD_BLINDING_MARKER: &str = "[WITHHELD_MARKET_BLINDING]";

const FORBIDDEN_BLIND_MARKERS: &[&str] = &[
    "{p_market}",
    "\"p_market\"",
    "p_market=",
    "p_market:",
    "crowd_prior",
    "forecaster_probability_distribution",
    "polymarket",
    "kalshi",
    "metaculus_community_median",
    "metaculus community",
];

/// Sole-match behavioral probes pinning every branch of `FORBIDDEN_BLIND_MARKERS` so a refactor
/// can never silently weaken or delete a blinding pattern without failing unit tests.
pub const BLIND_MARKER_SOLE_PROBES: &[(&str, &str)] = &[
    ("{p_market}", "template placeholder {p_market} leaked"),
    ("\"p_market\"", "wire json {\"p_market\": 0.42}"),
    ("p_market=", "url param ?p_market=0.72"),
    ("p_market:", "raw kv p_market: 0.42"),
    ("crowd_prior", "injected crowd_prior=0.55"),
    (
        "forecaster_probability_distribution",
        "leaked forecaster_probability_distribution field",
    ),
    ("polymarket", "live polymarket odds snapshot"),
    ("kalshi", "live kalshi orderbook quote"),
    (
        "metaculus_community_median",
        "backfilled metaculus_community_median value",
    ),
    ("metaculus community", "leaked metaculus community forecast"),
];

impl MarketBlindingScrubber {
    /// Hard-fails if any market-price / crowd-prior key survived into a pre-seal prompt payload.
    pub fn assert_blind_payload(payload: &str) -> Result<(), BlindingError> {
        let lower = payload.to_ascii_lowercase();
        for &marker in FORBIDDEN_BLIND_MARKERS {
            if lower.contains(marker) {
                return Err(BlindingError::MarketTokenLeak(marker));
            }
        }
        Ok(())
    }

    /// Whole-string withholding: if `text` contains any forbidden market/crowd marker, replaces
    /// the entire string with [`WITHHELD_BLINDING_MARKER`] (no partial splicing).
    #[must_use]
    pub fn withhold_whole_string_if_leaky(text: &str) -> String {
        if Self::assert_blind_payload(text).is_err() {
            WITHHELD_BLINDING_MARKER.to_owned()
        } else {
            text.to_owned()
        }
    }

    /// Verifies that every marker in `FORBIDDEN_BLIND_MARKERS` is the sole match for its
    /// pinned probe in `BLIND_MARKER_SOLE_PROBES`.
    #[must_use]
    pub fn verify_sole_match_probes() -> bool {
        if FORBIDDEN_BLIND_MARKERS.len() != BLIND_MARKER_SOLE_PROBES.len() {
            return false;
        }
        for &(expected_marker, probe_text) in BLIND_MARKER_SOLE_PROBES {
            let lower = probe_text.to_ascii_lowercase();
            let hits: Vec<&str> = FORBIDDEN_BLIND_MARKERS
                .iter()
                .copied()
                .filter(|m| lower.contains(m))
                .collect();
            if hits != vec![expected_marker] {
                return false;
            }
        }
        true
    }

    /// Scrubs lines referencing any forbidden market-price or crowd-prior marker from untrusted
    /// news/comment excerpts, using the exact unified `FORBIDDEN_BLIND_MARKERS` inventory.
    #[must_use]
    pub fn scrub_text(input: &str) -> String {
        input
            .lines()
            .filter(|line| Self::assert_blind_payload(line).is_ok())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_evidence_handles_and_blocks_market_leaks() {
        assert!(VerifiedEvidenceRef::parse("[ev:deadbeef#span1]").is_ok());
        assert!(VerifiedEvidenceRef::parse("http://raw.url").is_err());
        assert!(MarketBlindingScrubber::assert_blind_payload("evidence only").is_ok());
        assert!(MarketBlindingScrubber::assert_blind_payload("{\"p_market\": 0.42}").is_err());
        assert!(MarketBlindingScrubber::verify_sole_match_probes());
        assert_eq!(
            MarketBlindingScrubber::withhold_whole_string_if_leaky("note: crowd_prior=0.55"),
            WITHHELD_BLINDING_MARKER
        );
        assert_eq!(
            MarketBlindingScrubber::withhold_whole_string_if_leaky("clean advisory"),
            "clean advisory"
        );
        let scrubbed =
            MarketBlindingScrubber::scrub_text("CERT-EU advisory\nPolymarket at 42%\nGrid alert");
        assert_eq!(scrubbed, "CERT-EU advisory\nGrid alert");
    }
}
