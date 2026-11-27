//! Deterministic Geo/Macro/Cyber `DomainGate` and cross-venue `EquivalenceCluster` matcher (`GATE-12`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Target in-scope domains for `fcast` (`GATE-12`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetDomain {
    Geopolitics,
    Macroeconomics,
    Cybersecurity,
}

/// Reason a candidate contract was rejected by [`DomainGate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    Sports,
    PopCulture,
    SubHourCrypto,
    OutOfScope,
}

/// Classification outcome produced by [`DomainGate::classify`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum DomainDecision {
    Accept {
        domain: TargetDomain,
        confidence: f64,
        matched_signals: Vec<String>,
    },
    Reject {
        reason: RejectReason,
        matched_signals: Vec<String>,
    },
}

impl DomainDecision {
    #[must_use]
    pub const fn is_accepted(&self) -> bool {
        matches!(self, Self::Accept { .. })
    }
}

/// Candidate question/contract fed into [`DomainGate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainCandidate {
    pub question_id: String,
    pub title: String,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub venue: String,
}

impl DomainCandidate {
    #[must_use]
    pub fn new(
        question_id: impl Into<String>,
        title: impl Into<String>,
        venue: impl Into<String>,
    ) -> Self {
        Self {
            question_id: question_id.into(),
            title: title.into(),
            description: String::new(),
            category: None,
            tags: Vec::new(),
            venue: venue.into(),
        }
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Precision / recall / F1 summary over a labeled benchmark set.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkMetrics {
    pub tp: usize,
    pub fp: usize,
    pub tn: usize,
    pub fn_count: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

const SPORTS_TERMS: &[&str] = &[
    "nba",
    "nfl",
    "mlb",
    "nhl",
    "fifa",
    "uefa",
    "ncaa",
    "ufc",
    "pga",
    "wimbledon",
    "premier league",
    "champions league",
    "super bowl",
    "world cup",
    "world series",
    "stanley cup",
    "nba finals",
    "playoffs",
    "touchdown",
    "quarterback",
    "home run",
    "grand slam",
    "formula 1",
    "f1 race",
    "grand prix",
    "heavyweight bout",
    "boxing match",
    "tennis match",
    "soccer match",
    "football game",
    "basketball game",
    "baseball game",
    "hockey game",
    "olympic gold",
    "marathon",
    "mvp award",
    "ballon d or",
    "tour de france",
    "ryder cup",
    "cricket world",
    "rugby world",
    "la liga",
    "serie a",
    "bundesliga",
    "esports tournament",
];

const POP_CULTURE_TERMS: &[&str] = &[
    "oscar",
    "oscars",
    "academy award",
    "grammy",
    "grammys",
    "emmy",
    "emmys",
    "golden globe",
    "box office",
    "rotten tomatoes",
    "billboard hot 100",
    "taylor swift",
    "beyonce",
    "kardashian",
    "netflix top",
    "spotify streams",
    "album release",
    "movie sequel",
    "celebrity wedding",
    "celebrity breakup",
    "reality tv",
    "bachelor finale",
    "eurovision",
    "met gala",
    "coachella",
    "tiktok viral",
    "youtube subscriber",
    "twitch streamer",
    "k-pop",
    "kpop",
    "anime season",
    "video game awards",
    "gta 6",
    "fashion week",
    "tabloid",
    "pop star",
    "superstar tour",
];

const CRYPTO_ASSET_TERMS: &[&str] = &[
    "btc", "bitcoin", "eth", "ethereum", "sol", "solana", "doge", "dogecoin", "xrp", "ripple",
    "bnb", "ada", "avax", "memecoin", "altcoin", "crypto",
];

const SUB_HOUR_TERMS: &[&str] = &[
    "1m",
    "5m",
    "10m",
    "15m",
    "30m",
    "45m",
    "1h",
    "1-min",
    "5-min",
    "15-min",
    "30-min",
    "1-minute",
    "5-minute",
    "15-minute",
    "30-minute",
    "next 5 minutes",
    "next 15 minutes",
    "next 30 minutes",
    "next hour",
    "within 15 minutes",
    "within 30 minutes",
    "sub-hour",
    "hourly close",
    "15 minute",
    "5 minute",
    "30 minute",
    "up or down in",
];

const GEO_TERMS: &[&str] = &[
    "nato",
    "un security council",
    "ceasefire",
    "sanctions",
    "embargo",
    "treaty",
    "diplomatic",
    "border clash",
    "strait of hormuz",
    "taiwan strait",
    "south china sea",
    "baltic",
    "black sea",
    "red sea",
    "houthi",
    "ukraine",
    "russia",
    "kremlin",
    "beijing",
    "china",
    "taiwan",
    "iran",
    "israel",
    "gaza",
    "lebanon",
    "hezbollah",
    "north korea",
    "pyongyang",
    "missile",
    "ballistic",
    "nuclear",
    "coup",
    "election",
    "prime minister",
    "president",
    "parliament",
    "coalition government",
    "referendum",
    "troop",
    "military",
    "airstrike",
    "incursion",
    "blockade",
    "peacekeeping",
    "subsea cable",
    "arctic",
    "suwalki",
    "armistice",
    "annexation",
    "geopolitics",
    "foreign policy",
    "defense pact",
];

const MACRO_TERMS: &[&str] = &[
    "federal reserve",
    "fomc",
    "fed funds",
    "rate cut",
    "rate hike",
    "interest rate",
    "central bank",
    "ecb",
    "bank of japan",
    "boj",
    "bank of england",
    "pboc",
    "cpi",
    "pce",
    "inflation",
    "deflation",
    "gdp",
    "recession",
    "unemployment",
    "nonfarm payroll",
    "payroll",
    "treasury yield",
    "10-year yield",
    "yield curve",
    "sovereign debt",
    "default",
    "debt ceiling",
    "tariff",
    "trade war",
    "trade deficit",
    "opec",
    "brent crude",
    "wti crude",
    "oil price",
    "natural gas",
    "ttf gas",
    "imf",
    "world bank",
    "exchange rate",
    "usd jpy",
    "eur usd",
    "currency peg",
    "pmi",
    "industrial production",
    "fiscal deficit",
    "quantitative tightening",
    "quantitative easing",
    "macroeconomics",
    "economics",
    "monetary policy",
];

const CYBER_TERMS: &[&str] = &[
    "ransomware",
    "zero-day",
    "zero day",
    "0-day",
    "cve",
    "cisa",
    "kev",
    "epss",
    "apt",
    "apt28",
    "apt29",
    "apt41",
    "volt typhoon",
    "salt typhoon",
    "lazarus group",
    "sandworm",
    "ddos",
    "bgp hijack",
    "bgp",
    "dns outage",
    "cloudflare radar",
    "ioda",
    "internet shutdown",
    "critical infrastructure",
    "power grid",
    "scada",
    "ics",
    "supply chain attack",
    "data breach",
    "cyberattack",
    "cyber attack",
    "cybersecurity",
    "malware",
    "wiper",
    "botnet",
    "espionage",
    "backdoor",
    "exploit",
    "vulnerability",
    "telecom intrusion",
    "satellite jamming",
    "undersea cable sabotage",
    "eurepoc",
    "vcdb",
    "intrusion",
    "nation-state hack",
];

/// Deterministic Geo/Macro/Cyber domain filter (`GATE-12`).
#[derive(Debug, Clone, Copy, Default)]
pub struct DomainGate;

impl DomainGate {
    /// Classify a [`DomainCandidate`] deterministically into `Accept(Geo|Macro|Cyber)`
    /// or `Reject(Sports|PopCulture|SubHourCrypto|OutOfScope)`.
    #[must_use]
    pub fn classify(&self, candidate: &DomainCandidate) -> DomainDecision {
        let mut combined =
            String::with_capacity(candidate.title.len() + candidate.description.len() + 64);
        combined.push_str(&candidate.title);
        combined.push(' ');
        combined.push_str(&candidate.description);
        if let Some(cat) = &candidate.category {
            combined.push(' ');
            combined.push_str(cat);
        }
        for tag in &candidate.tags {
            combined.push(' ');
            combined.push_str(tag);
        }
        let norm = normalize_for_matching(&combined);

        // 1. Hard rejection filters: sub-hour crypto, sports, pop-culture.
        let crypto_hits = matched_terms(&norm, CRYPTO_ASSET_TERMS);
        let sub_hour_hits = matched_terms(&norm, SUB_HOUR_TERMS);
        if !crypto_hits.is_empty() && !sub_hour_hits.is_empty() {
            let mut signals = crypto_hits;
            signals.extend(sub_hour_hits);
            return DomainDecision::Reject {
                reason: RejectReason::SubHourCrypto,
                matched_signals: signals,
            };
        }

        let sports_hits = matched_terms(&norm, SPORTS_TERMS);
        let pop_hits = matched_terms(&norm, POP_CULTURE_TERMS);

        let geo_hits = matched_terms(&norm, GEO_TERMS);
        let macro_hits = matched_terms(&norm, MACRO_TERMS);
        let cyber_hits = matched_terms(&norm, CYBER_TERMS);

        let best_in_scope = geo_hits.len().max(macro_hits.len()).max(cyber_hits.len());

        if !sports_hits.is_empty() && sports_hits.len() >= best_in_scope {
            return DomainDecision::Reject {
                reason: RejectReason::Sports,
                matched_signals: sports_hits,
            };
        }
        if !pop_hits.is_empty() && pop_hits.len() >= best_in_scope {
            return DomainDecision::Reject {
                reason: RejectReason::PopCulture,
                matched_signals: pop_hits,
            };
        }

        // 2. Select highest-scoring in-scope domain (tie-break: Cyber > Macro > Geo).
        if best_in_scope == 0 {
            return DomainDecision::Reject {
                reason: RejectReason::OutOfScope,
                matched_signals: Vec::new(),
            };
        }

        let (domain, signals) =
            if cyber_hits.len() >= macro_hits.len() && cyber_hits.len() >= geo_hits.len() {
                (TargetDomain::Cybersecurity, cyber_hits)
            } else if macro_hits.len() >= geo_hits.len() {
                (TargetDomain::Macroeconomics, macro_hits)
            } else {
                (TargetDomain::Geopolitics, geo_hits)
            };

        let confidence = (0.55 + 0.15 * (signals.len() as f64)).min(0.99);
        DomainDecision::Accept {
            domain,
            confidence,
            matched_signals: signals,
        }
    }

    /// Evaluate precision, recall, and F1 over a labeled benchmark dataset.
    #[must_use]
    pub fn evaluate_benchmark(&self, labeled: &[(DomainCandidate, bool)]) -> BenchmarkMetrics {
        let mut tp = 0usize;
        let mut fp = 0usize;
        let mut tn = 0usize;
        let mut fn_count = 0usize;
        for (cand, expected_accept) in labeled {
            let accepted = self.classify(cand).is_accepted();
            match (accepted, *expected_accept) {
                (true, true) => tp += 1,
                (true, false) => fp += 1,
                (false, false) => tn += 1,
                (false, true) => fn_count += 1,
            }
        }
        let precision = if tp + fp > 0 {
            (tp as f64) / ((tp + fp) as f64)
        } else {
            1.0
        };
        let recall = if tp + fn_count > 0 {
            (tp as f64) / ((tp + fn_count) as f64)
        } else {
            1.0
        };
        let f1 = if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        };
        BenchmarkMetrics {
            tp,
            fp,
            tn,
            fn_count,
            precision,
            recall,
            f1,
        }
    }
}

fn normalize_for_matching(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push(' ');
    let mut prev_space = true;
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '-' {
            out.push(lower);
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    if !prev_space {
        out.push(' ');
    }
    out
}

fn matched_terms(norm_padded: &str, terms: &[&str]) -> Vec<String> {
    let mut hits = Vec::new();
    for &term in terms {
        let needle = format!(" {term} ");
        if norm_padded.contains(&needle) {
            hits.push(term.to_string());
        }
    }
    hits
}

/// Reference to a single venue's market/question contract for cross-venue clustering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketContractRef {
    pub venue: String,
    pub question_id: String,
    pub title: String,
    pub resolution_date_utc: Option<String>,
    pub domain: TargetDomain,
}

/// Cross-venue equivalence cluster aligning contracts referring to the same underlying event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquivalenceCluster {
    pub cluster_id: String,
    pub canonical_title: String,
    pub domain: TargetDomain,
    pub members: Vec<MarketContractRef>,
    pub min_pair_similarity: f64,
}

/// Deterministic cross-venue [`EquivalenceCluster`] matcher (`GATE-12`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClusterMatcher {
    pub min_jaccard: f64,
    pub max_date_diff_days: u32,
}

impl Default for ClusterMatcher {
    fn default() -> Self {
        Self {
            min_jaccard: 0.45,
            max_date_diff_days: 14,
        }
    }
}

impl ClusterMatcher {
    /// Group contracts across venues into deterministic [`EquivalenceCluster`]s.
    #[must_use]
    pub fn cluster(&self, contracts: &[MarketContractRef]) -> Vec<EquivalenceCluster> {
        let mut sorted: Vec<MarketContractRef> = contracts.to_vec();
        sorted.sort_by(|a, b| {
            a.domain
                .cmp(&b.domain)
                .then_with(|| a.venue.cmp(&b.venue))
                .then_with(|| a.question_id.cmp(&b.question_id))
        });

        let token_sets: Vec<BTreeSet<String>> =
            sorted.iter().map(|c| canonical_tokens(&c.title)).collect();
        let days: Vec<Option<i64>> = sorted
            .iter()
            .map(|c| c.resolution_date_utc.as_deref().and_then(parse_ymd_days))
            .collect();

        let n = sorted.len();
        let mut visited = vec![false; n];
        let mut clusters = Vec::new();

        for i in 0..n {
            if visited[i] {
                continue;
            }
            visited[i] = true;
            let mut member_indices = vec![i];
            let mut min_sim = 1.0_f64;

            for j in (i + 1)..n {
                if visited[j] || sorted[j].domain != sorted[i].domain {
                    continue;
                }
                if let (Some(d1), Some(d2)) = (days[i], days[j])
                    && (d1 - d2).unsigned_abs() > u64::from(self.max_date_diff_days)
                {
                    continue;
                }
                let sim = jaccard_similarity(&token_sets[i], &token_sets[j]);
                if sim >= self.min_jaccard {
                    visited[j] = true;
                    member_indices.push(j);
                    min_sim = min_sim.min(sim);
                }
            }

            let members: Vec<MarketContractRef> = member_indices
                .iter()
                .map(|&idx| sorted[idx].clone())
                .collect();
            let canonical_title = members[0].title.clone();
            let domain = members[0].domain;
            let mut sig_tokens: Vec<String> = token_sets[i].iter().take(4).cloned().collect();
            if sig_tokens.is_empty() {
                sig_tokens.push(members[0].question_id.clone());
            }
            let cluster_id = format!("clu_{}_{}", domain_slug(domain), sig_tokens.join("_"));
            clusters.push(EquivalenceCluster {
                cluster_id,
                canonical_title,
                domain,
                members,
                min_pair_similarity: min_sim,
            });
        }

        // Deduplicate cluster_ids deterministically if two clusters share the same 4-token prefix.
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for clu in &mut clusters {
            let entry = counts.entry(clu.cluster_id.clone()).or_insert(0);
            *entry += 1;
            if *entry > 1 {
                clu.cluster_id = format!("{}_{}", clu.cluster_id, *entry);
            }
        }
        clusters
    }
}

const fn domain_slug(d: TargetDomain) -> &'static str {
    match d {
        TargetDomain::Geopolitics => "geo",
        TargetDomain::Macroeconomics => "macro",
        TargetDomain::Cybersecurity => "cyber",
    }
}

fn canonical_tokens(title: &str) -> BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "will", "the", "a", "an", "in", "on", "at", "by", "of", "to", "for", "be", "is", "are",
        "before", "after", "during", "above", "below", "over", "under", "or", "and", "with",
        "from", "that", "this", "it", "its", "as", "into", "end",
    ];
    let norm = normalize_for_matching(title);
    let mut set = BTreeSet::new();
    for raw in norm.split_whitespace() {
        let tok = raw.trim_matches('-');
        if tok.len() < 2 || STOPWORDS.contains(&tok) {
            continue;
        }
        let canon = match tok {
            "fomc" | "fed" | "reserve" => "fed",
            "federal" => continue,
            "cuts" | "cutting" | "lower" | "reduce" => "cut",
            "hikes" | "hiking" | "raise" | "raises" => "hike",
            "rates" => "rate",
            "bps" | "basis" => "bps",
            "u-s" | "usa" | "united" | "states" => "us",
            "prc" | "beijing" | "chinese" => "china",
            "russian" | "kremlin" => "russia",
            "ukrainian" => "ukraine",
            "iranian" | "tehran" => "iran",
            "israeli" => "israel",
            "inflation" => "cpi",
            "cyberattack" | "cyber-attack" => "cyberattack",
            other => other,
        };
        set.insert(canon.to_string());
    }
    set
}

fn jaccard_similarity(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        (inter as f64) / (union as f64)
    }
}

/// Parse a `YYYY-MM-DD` prefix into an exact proleptic Gregorian day count via `fcast_core::time`.
fn parse_ymd_days(ts: &str) -> Option<i64> {
    fcast_core::time::parse_ymd_days(ts)
}
