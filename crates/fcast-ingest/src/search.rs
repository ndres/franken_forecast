//! Milestone `M2` Multi-Provider Parallel Search Aggregator & Novelty Deduplicator (`SUB-02`).
//!
//! Fans out read-only queries across multiple search providers concurrently via `std::thread::scope`,
//! enforces `INV-03` (`available_utc <= as_of_utc`) temporal fencing, scrubs market-price leaks via
//! [`MarketBlindingScrubber`], deduplicates near-identical documents (computing a clipped novelty
//! score in `[0, 1]`), and vaults surviving evidence spans into [`EvidenceVault`].

use fcast_core::blinding::{MarketBlindingScrubber, VerifiedEvidenceRef};
use fcast_core::time::{is_utc_le, parse_utc_epoch_ms};
use fcast_infra::vault::{EvidenceVault, VaultError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("provider {provider} failed: {message}")]
    Provider { provider: String, message: String },
    #[error("invalid as_of_utc timestamp for search: {0}")]
    InvalidTimestamp(String),
    #[error("vault error while storing search hit: {0}")]
    Vault(#[from] VaultError),
}

/// Raw document returned by a single search provider before fencing/blinding/vaulting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDocument {
    pub provider: String,
    pub canonical_url: String,
    pub title: String,
    pub body_excerpt: String,
    pub published_utc: String,
    pub available_utc: String,
}

/// Time-fenced, market-blinded, deduplicated, and vaulted search result ready for `Loom`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultedSearchHit {
    pub provider: String,
    pub canonical_url: String,
    pub title: String,
    pub available_utc: String,
    pub ev_handle: VerifiedEvidenceRef,
    pub sha256_hex: String,
    /// Novelty score in `[0.0, 1.0]` relative to earlier retained hits (`1.0` = completely novel).
    pub novelty: f64,
}

/// Summary report returned by [`MultiProviderSearchAggregator::search_and_vault`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchAggregationReport {
    pub hits: Vec<VaultedSearchHit>,
    pub dropped_post_cutoff: usize,
    pub dropped_duplicates: usize,
    pub provider_errors: Vec<String>,
}

/// Pure-Rust sync search provider trait (offline-mockable, zero banned async runtimes).
pub trait SearchProvider: Sync {
    fn name(&self) -> &str;
    fn query(&self, query: &str, as_of_utc: &str) -> Result<Vec<SearchDocument>, String>;
}

/// Parallel multi-provider search aggregator with temporal fencing, blinding, and novelty dedup.
#[derive(Debug, Clone, Copy)]
pub struct MultiProviderSearchAggregator {
    /// Jaccard token similarity at or above which a document is dropped as a duplicate (`0.75`).
    pub duplicate_jaccard_threshold: f64,
}

impl Default for MultiProviderSearchAggregator {
    fn default() -> Self {
        Self {
            duplicate_jaccard_threshold: 0.75,
        }
    }
}

impl MultiProviderSearchAggregator {
    /// Executes all `providers` concurrently, enforces `available_utc <= as_of_utc`, scrubs
    /// pre-seal market leaks, deduplicates by URL + token Jaccard, and vaults every survivor.
    pub fn search_and_vault(
        &self,
        providers: &[&dyn SearchProvider],
        query: &str,
        as_of_utc: &str,
        vault: &EvidenceVault,
    ) -> Result<SearchAggregationReport, SearchError> {
        parse_utc_epoch_ms(as_of_utc)
            .map_err(|_| SearchError::InvalidTimestamp(as_of_utc.to_owned()))?;

        let mut all_docs: Vec<SearchDocument> = Vec::new();
        let mut provider_errors: Vec<String> = Vec::new();

        std::thread::scope(|scope| {
            let handles: Vec<_> = providers
                .iter()
                .map(|&p| scope.spawn(move || (p.name().to_owned(), p.query(query, as_of_utc))))
                .collect();

            for h in handles {
                match h.join() {
                    Ok((_, Ok(docs))) => all_docs.extend(docs),
                    Ok((name, Err(msg))) => provider_errors.push(format!("{name}: {msg}")),
                    Err(_) => provider_errors.push("provider thread panicked".to_owned()),
                }
            }
        });

        // Deterministic epoch-ms ordering before deduplication so concurrent arrival order and
        // timestamp representations ('.500Z' vs 'Z' / offsets) never flake dedup winners.
        all_docs.sort_by(|a, b| {
            let ms_a = parse_utc_epoch_ms(&a.available_utc).unwrap_or(i64::MAX);
            let ms_b = parse_utc_epoch_ms(&b.available_utc).unwrap_or(i64::MAX);
            ms_a.cmp(&ms_b)
                .then_with(|| {
                    a.canonical_url
                        .trim()
                        .to_ascii_lowercase()
                        .cmp(&b.canonical_url.trim().to_ascii_lowercase())
                })
                .then_with(|| a.provider.cmp(&b.provider))
        });

        let mut seen_urls: BTreeSet<String> = BTreeSet::new();
        let mut seen_token_sets: Vec<BTreeSet<String>> = Vec::new();
        let mut hits = Vec::new();
        let mut dropped_post_cutoff = 0usize;
        let mut dropped_duplicates = 0usize;

        for doc in all_docs {
            if !is_utc_le(&doc.available_utc, as_of_utc)
                || !is_utc_le(&doc.published_utc, as_of_utc)
            {
                dropped_post_cutoff += 1;
                continue;
            }
            let norm_url = doc.canonical_url.trim().to_ascii_lowercase();
            // Reject market-blinding URLs (e.g. polymarket.com, kalshi.com, ?p_market=) & control chars
            if norm_url.is_empty()
                || doc
                    .canonical_url
                    .bytes()
                    .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
                || MarketBlindingScrubber::assert_blind_payload(&norm_url).is_err()
            {
                dropped_duplicates += 1;
                continue;
            }
            if !seen_urls.insert(norm_url) {
                dropped_duplicates += 1;
                continue;
            }

            let scrubbed_body = MarketBlindingScrubber::scrub_text(&doc.body_excerpt);
            let scrubbed_title = MarketBlindingScrubber::scrub_text(&doc.title);
            if scrubbed_title.trim().is_empty() && scrubbed_body.trim().is_empty() {
                continue;
            }
            let tokens = doc_tokens(&scrubbed_title, &scrubbed_body);
            let max_sim = seen_token_sets
                .iter()
                .map(|prev| token_jaccard(prev, &tokens))
                .fold(0.0_f64, f64::max);
            if max_sim >= self.duplicate_jaccard_threshold {
                dropped_duplicates += 1;
                continue;
            }
            let novelty = (1.0 - max_sim).clamp(0.0, 1.0);
            seen_token_sets.push(tokens);

            let span_id = format!("src_{}", hits.len());
            let payload = format!(
                "URL: {}\nTitle: {}\nPublished: {}\n\n{}",
                doc.canonical_url.trim(),
                scrubbed_title,
                doc.published_utc.trim(),
                scrubbed_body
            );
            if MarketBlindingScrubber::assert_blind_payload(&payload).is_err() {
                continue;
            }
            let art = vault.put_bytes(&span_id, payload.as_bytes())?;
            hits.push(VaultedSearchHit {
                provider: doc.provider,
                canonical_url: doc.canonical_url,
                title: scrubbed_title,
                available_utc: doc.available_utc,
                ev_handle: art.ev_handle,
                sha256_hex: art.sha256_hex,
                novelty,
            });
        }

        Ok(SearchAggregationReport {
            hits,
            dropped_post_cutoff,
            dropped_duplicates,
            provider_errors,
        })
    }
}

fn doc_tokens(title: &str, body: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for raw in title.split_whitespace().chain(body.split_whitespace()) {
        let cleaned: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        let char_count = cleaned.chars().count();
        if char_count >= 3 {
            set.insert(cleaned);
        } else if char_count >= 1 && !cleaned.is_ascii() {
            // Preserve short non-ASCII (e.g. CJK) words as distinct tokens
            set.insert(cleaned);
        }
    }
    set
}

fn token_jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        (inter as f64) / (union as f64)
    }
}
