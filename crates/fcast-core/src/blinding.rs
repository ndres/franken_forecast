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

const FORBIDDEN_BLIND_MARKERS: &[&str] = &[
    "{p_market}",
    "\"p_market\"",
    "crowd_prior",
    "forecaster_probability_distribution",
    "polymarket_price",
    "kalshi_price",
    "metaculus_community_median",
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

    /// Scrubs lines referencing market prices or crowd odds from untrusted news/comment excerpts.
    #[must_use]
    pub fn scrub_text(input: &str) -> String {
        input
            .lines()
            .filter(|line| {
                let l = line.to_ascii_lowercase();
                !(l.contains("polymarket")
                    || l.contains("kalshi")
                    || l.contains("metaculus community")
                    || l.contains("p_market")
                    || l.contains("crowd_prior"))
            })
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
        let scrubbed =
            MarketBlindingScrubber::scrub_text("CERT-EU advisory\nPolymarket at 42%\nGrid alert");
        assert_eq!(scrubbed, "CERT-EU advisory\nGrid alert");
    }
}
