//! Bi-temporal event verification, canonical prompt template catalog (`§9`),
//! and 4-rung `ValueExtractionLadder` + `THIN_PUBLISH_FLOOR` (`§11.2`, `GATE-08`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::forecast::ResolutionStatusAtCutoff;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Default normal binary publish bounds (`§11.2`, `OQ-04`).
pub const NORMAL_PUBLISH_BOUNDS: (f64, f64) = (0.02, 0.98);
/// Single-survivor degraded ensemble floor (`THIN_PUBLISH_FLOOR`, `§11.2`).
pub const THIN_PUBLISH_FLOOR: (f64, f64) = (0.05, 0.95);

/// Errors emitted by `TemplateCatalog`, `BiTemporalAudit`, and `ValueExtractionLadder`.
#[derive(Debug, Error, PartialEq)]
pub enum VerifyError {
    #[error("failed to read template {file}: {message}")]
    TemplateIo { file: String, message: String },
    #[error("SHA-256 digest mismatch for {file}: expected {expected}, got {actual}")]
    TemplateDigestMismatch {
        file: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error(
        "mechanical blinding invariant violated in {file}: forbidden placeholder {placeholder}"
    )]
    TemplateBlindingViolation {
        file: &'static str,
        placeholder: &'static str,
    },
    #[error("temporal fence violation: available_utc ({available_utc}) > as_of_utc ({as_of_utc})")]
    TemporalFenceViolation {
        available_utc: String,
        as_of_utc: String,
    },
    #[error("invalid UTC timestamp {0:?}: expected YYYY-MM-DD HH:MM or RFC-3339 UTC")]
    InvalidTimestamp(String),
    #[error("duplicate JSON key detected: {0}")]
    DuplicateJsonKey(String),
    #[error("boolean coerced as number for field {0}")]
    BooleanAsNumber(String),
    #[error("truncated or malformed JSON/numeric literal: {0}")]
    TruncatedOrInvalidLiteral(String),
    #[error("extracted probability {0} is outside [0.0, 1.0] or non-finite")]
    ProbabilityOutOfRange(f64),
    #[error("missing or unknown option in multiset: {0}")]
    OptionMultisetMismatch(String),
    #[error("value extraction failed across all 4 ladder rungs: {0}")]
    ExtractionExhausted(String),
}

/// Canonical identifier for the 9 SHA-256-locked prompt templates (`§9.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateId {
    BitemporalEventAudit,
    RationaleStagedTurnA,
    RationaleStagedTurnB,
    StreamLikelihoodTick,
    BeliefFoldStep,
    RedTeamAudit,
    SeiphProcessCritic,
    MetaForecastTriage,
    PostmortemEeDistill,
}

/// Static specification for one locked prompt template in `templates/*.tmpl` (`§9.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateSpec {
    pub id: TemplateId,
    pub filename: &'static str,
    pub expected_sha256: &'static str,
    /// Placeholders that MUST NEVER appear in the template (`INV-04`, `INV-05`, outcome blinding).
    pub forbidden_placeholders: &'static [&'static str],
    /// Placeholders that MUST appear in the template.
    pub required_placeholders: &'static [&'static str],
}

/// All 9 locked `templates/*.tmpl` specifications matching `§9.2` byte-for-byte.
pub const LOCKED_TEMPLATE_SPECS: [TemplateSpec; 9] = [
    TemplateSpec {
        id: TemplateId::BitemporalEventAudit,
        filename: "bitemporal_event_audit.tmpl",
        expected_sha256: "362d5386c85123d3fd491ee7d20da87c6055fa944a7b138253fc7d214b5d1f52",
        forbidden_placeholders: &["{p_market}"],
        required_placeholders: &[
            "{as_of_utc}",
            "{question_contracts}",
            "{union_strictness_hint}",
        ],
    },
    TemplateSpec {
        id: TemplateId::RationaleStagedTurnA,
        filename: "rationale_staged_turn_a.tmpl",
        expected_sha256: "54eb0539d2addfb4747fd74ab4e558f1bf92e561b0e1f733241a889f437f888d",
        forbidden_placeholders: &[
            "{p_market}",
            "{forecaster_probability_distribution}",
            "{crowd_prior}",
            "{forecaster_forecast}",
        ],
        required_placeholders: &[
            "{question_id}",
            "{as_of_utc}",
            "{contract_sha256}",
            "{rationale_handle}",
            "{rationale_text}",
        ],
    },
    TemplateSpec {
        id: TemplateId::RationaleStagedTurnB,
        filename: "rationale_staged_turn_b.tmpl",
        expected_sha256: "1dffea0e33a600577fe4282dc170f452488d09187f58714bba4016ae9feaca7f",
        forbidden_placeholders: &["{question_resolution_json}"],
        required_placeholders: &[
            "{locked_turn_a_json}",
            "{forecaster_probability_distribution}",
            "{p_market}",
            "{as_of_utc}",
        ],
    },
    TemplateSpec {
        id: TemplateId::StreamLikelihoodTick,
        filename: "stream_likelihood_tick.tmpl",
        expected_sha256: "79be8dc01439c55213170757702eea2c18928d82958bb592b7924c27e0b476e7",
        forbidden_placeholders: &["{p_market}", "{crowd_prior}"],
        required_placeholders: &[
            "{forecast_id}",
            "{question_id}",
            "{as_of_utc}",
            "{horizon_geometry}",
            "{cluster_novelty}",
        ],
    },
    TemplateSpec {
        id: TemplateId::BeliefFoldStep,
        filename: "belief_fold_step.tmpl",
        expected_sha256: "da7f1f3480c581e43f4a47f8720338df80096f741424f461d1a665da9d950797",
        forbidden_placeholders: &["{p_market}", "{crowd_prior}"],
        required_placeholders: &[
            "{forecast_id}",
            "{parent_revision_id}",
            "{seq_no}",
            "{as_of_utc}",
            "{cumulative_tombstone_handles}",
        ],
    },
    TemplateSpec {
        id: TemplateId::RedTeamAudit,
        filename: "red_team_audit.tmpl",
        expected_sha256: "061613585fb855e40a905933ab57bb8ce4ace7a33f670e4e562c954cb527b423",
        forbidden_placeholders: &["{p_market}", "{crowd_prior}"],
        required_placeholders: &[
            "{gold_dossier_handle}",
            "{proposed_revision_delta_json}",
            "{ee_and_cass_guardrails}",
            "{as_of_utc}",
        ],
    },
    TemplateSpec {
        id: TemplateId::SeiphProcessCritic,
        filename: "seiph_process_critic.tmpl",
        expected_sha256: "83ce2e19d2e6b2d667af66a8a619f8f8410eb54dfc6b4e4020f92a30a8f1176e",
        forbidden_placeholders: &["{question_resolution_json}"],
        required_placeholders: &[
            "{revision_id}",
            "{question_id}",
            "{as_of_utc}",
            "{p_indep}",
            "{p_market}",
            "{rationale_handle}",
        ],
    },
    TemplateSpec {
        id: TemplateId::MetaForecastTriage,
        filename: "meta_forecast_triage.tmpl",
        expected_sha256: "3f797bdec2e3bdc53c34a96806fb8caebb58d7364a2c6ed581e75928556a3dbe",
        forbidden_placeholders: &["{p_market}", "{question_resolution_json}"],
        required_placeholders: &[
            "{question_id}",
            "{discovered_utc}",
            "{dist_kind}",
            "{horizon_geometry}",
        ],
    },
    TemplateSpec {
        id: TemplateId::PostmortemEeDistill,
        filename: "postmortem_ee_distill.tmpl",
        expected_sha256: "0d24a7c7c55619cea60058a8dcc1d0e5fecb679fe46de865182064b84a3c04fc",
        forbidden_placeholders: &[],
        required_placeholders: &[
            "{origin_revision_id}",
            "{initial_forecast_json}",
            "{subsequent_revisions_json}",
            "{horizon_scores_json}",
            "{question_resolution_json}",
        ],
    },
];

/// Verified, SHA-256-locked template artifact loaded into memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedTemplate {
    pub id: TemplateId,
    pub filename: &'static str,
    pub sha256_hex: String,
    pub body: String,
}

impl LoadedTemplate {
    /// Short 8-hex-char tag for `ForecastRevision.model_version` (`+tmpl:<8hex>`, `§9`).
    #[must_use]
    pub fn short_tag(&self) -> String {
        format!("tmpl:{}", &self.sha256_hex[..8])
    }
}

/// Catalog of all 9 locked `templates/*.tmpl` artifacts with SHA-256 and blinding verification.
#[derive(Debug, Clone)]
pub struct TemplateCatalog {
    templates: BTreeMap<TemplateId, LoadedTemplate>,
}

impl TemplateCatalog {
    /// Loads all 9 `templates/*.tmpl` from `templates_dir`, verifying exact SHA-256 digests
    /// and mechanical variable-level blinding invariants (`INV-04`, `INV-05`).
    pub fn load_and_verify(templates_dir: &Path) -> Result<Self, VerifyError> {
        let mut templates = BTreeMap::new();
        for spec in LOCKED_TEMPLATE_SPECS {
            let path = templates_dir.join(spec.filename);
            let body = fs::read_to_string(&path).map_err(|e| VerifyError::TemplateIo {
                file: path.display().to_string(),
                message: e.to_string(),
            })?;
            let actual = sha256_hex(body.as_bytes());
            if actual != spec.expected_sha256 {
                return Err(VerifyError::TemplateDigestMismatch {
                    file: spec.filename,
                    expected: spec.expected_sha256,
                    actual,
                });
            }
            Self::verify_blinding(&spec, &body)?;
            templates.insert(
                spec.id,
                LoadedTemplate {
                    id: spec.id,
                    filename: spec.filename,
                    sha256_hex: actual,
                    body,
                },
            );
        }
        Ok(Self { templates })
    }

    /// Checks a template body against its forbidden and required placeholder variables.
    pub fn verify_blinding(spec: &TemplateSpec, body: &str) -> Result<(), VerifyError> {
        for &forbidden in spec.forbidden_placeholders {
            if body.contains(forbidden) {
                return Err(VerifyError::TemplateBlindingViolation {
                    file: spec.filename,
                    placeholder: forbidden,
                });
            }
        }
        for &required in spec.required_placeholders {
            if !body.contains(required) {
                return Err(VerifyError::TemplateBlindingViolation {
                    file: spec.filename,
                    placeholder: required,
                });
            }
        }
        Ok(())
    }

    /// Returns the verified template for `id`.
    #[must_use]
    pub fn get(&self, id: TemplateId) -> Option<&LoadedTemplate> {
        self.templates.get(&id)
    }

    /// Number of loaded templates (always 9 for a verified catalog).
    #[must_use]
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// Iterates over all loaded templates in canonical enum order.
    pub fn iter(&self) -> impl Iterator<Item = (&TemplateId, &LoadedTemplate)> {
        self.templates.iter()
    }
}

/// Computes lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

// ---------------------------------------------------------------------------
// BiTemporalAudit (§9.1, GATE-08)
// ---------------------------------------------------------------------------

/// A question contract submitted to `BiTemporalAudit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionContractSpec {
    pub question_id: String,
    pub venue: String,
    pub contract_sha256: String,
    pub rules_summary: String,
}

/// One candidate evidence item evaluated during bi-temporal event auditing (`§9.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvidenceItem {
    pub ev_handle: Option<VerifiedEvidenceRef>,
    /// Public visibility timestamp (`YYYY-MM-DD HH:MM` or RFC-3339 UTC).
    pub published_utc: String,
    /// Optional physical event timestamp (`event_utc`).
    pub event_utc: Option<String>,
    /// Vault ingestion/archive availability timestamp (`available_utc <= as_of_utc`).
    pub available_utc: String,
    pub who_or_outlet: String,
    pub source_type: String,
    pub satisfies_rules: bool,
    /// Which `question_id`s in `question_contracts` this item satisfies.
    pub qualified_question_ids: Vec<String>,
    pub quote: String,
}

/// Per-question resolution projection produced by `BiTemporalAudit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerQuestionResolution {
    pub question_id: String,
    pub resolution_status: ResolutionStatusAtCutoff,
    pub resolved_published_utc: Option<String>,
    pub adjudication_ev_handle: Option<VerifiedEvidenceRef>,
}

/// Output of `BiTemporalAudit::audit` (`§9.1`, `GATE-08`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiTemporalAuditOutcome {
    pub as_of_utc: String,
    /// Earliest `published_utc <= as_of_utc` among items satisfying at least one contract
    /// in `question_contracts` (union-strictness), or `None` when `Q = empty` or zero items qualify.
    pub first_public: Option<AuditEvidenceItem>,
    pub second_source: Option<AuditEvidenceItem>,
    pub earlier_signals: Vec<AuditEvidenceItem>,
    pub per_question_resolution: Vec<PerQuestionResolution>,
}

/// Parsed millisecond-precision UTC instant for exact bi-temporal ordering and `[HH:MM:00Z, HH:MM:59.999Z]` joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcMillis(pub i64);

/// Inclusive `[HH:MM:00.000Z, HH:MM:59.999Z]` minute window for lead/lag joins (`§9.2`, `GATE-08`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinuteWindow {
    pub start_ms: UtcMillis,
    pub end_ms: UtcMillis,
}

impl MinuteWindow {
    /// Returns `true` iff `ts` falls inside `[HH:MM:00.000Z, HH:MM:59.999Z]`.
    #[must_use]
    pub fn contains(self, ts: UtcMillis) -> bool {
        ts >= self.start_ms && ts <= self.end_ms
    }

    /// Lead/lag delta in milliseconds relative to the window:
    /// - `< 0`: `ts` precedes `start_ms` (lead)
    /// - `0`: `ts` lies inside `[HH:MM:00.000Z, HH:MM:59.999Z]` (simultaneous minute join)
    /// - `> 0`: `ts` follows `end_ms` (lag)
    #[must_use]
    pub fn signed_offset_ms(self, ts: UtcMillis) -> i64 {
        if ts.0 < self.start_ms.0 {
            ts.0 - self.start_ms.0
        } else if ts.0 > self.end_ms.0 {
            ts.0 - self.end_ms.0
        } else {
            0
        }
    }
}

/// Bi-temporal event auditor enforcing causal fencing, union-strictness, `Q = empty` nullability,
/// and minute-window lead/lag joins (`§9.1`, `GATE-08`).
#[derive(Debug, Default, Clone, Copy)]
pub struct BiTemporalAudit;

impl BiTemporalAudit {
    /// Enforces `available_utc <= as_of_utc`, union-strictness `first_public` selection across
    /// `question_contracts`, and `Q = empty` nullability.
    pub fn audit(
        as_of_utc: &str,
        question_contracts: &[QuestionContractSpec],
        items: &[AuditEvidenceItem],
    ) -> Result<BiTemporalAuditOutcome, VerifyError> {
        let cutoff = parse_utc_millis(as_of_utc)?;

        // Enforce strict bi-temporal fence on every candidate item: available_utc <= as_of_utc.
        let mut parsed_items: Vec<(UtcMillis, &AuditEvidenceItem)> =
            Vec::with_capacity(items.len());
        for item in items {
            let avail = parse_utc_millis(&item.available_utc)?;
            if avail > cutoff {
                return Err(VerifyError::TemporalFenceViolation {
                    available_utc: item.available_utc.clone(),
                    as_of_utc: as_of_utc.to_owned(),
                });
            }
            let published = parse_utc_millis(&item.published_utc)?;
            parsed_items.push((published, item));
        }

        // Q = empty => first_public is strictly None and per_question_resolution is empty.
        if question_contracts.is_empty() {
            return Ok(BiTemporalAuditOutcome {
                as_of_utc: as_of_utc.to_owned(),
                first_public: None,
                second_source: None,
                earlier_signals: Vec::new(),
                per_question_resolution: Vec::new(),
            });
        }

        let contract_ids: BTreeSet<&str> = question_contracts
            .iter()
            .map(|c| c.question_id.as_str())
            .collect();

        // Union-strictness: an item qualifies if published_utc <= as_of_utc, satisfies_rules == true,
        // and matches at least one question_id in question_contracts (the least-strict union).
        let mut qualifying: Vec<(UtcMillis, &AuditEvidenceItem)> = parsed_items
            .iter()
            .copied()
            .filter(|(pub_ms, item)| {
                *pub_ms <= cutoff
                    && item.satisfies_rules
                    && item
                        .qualified_question_ids
                        .iter()
                        .any(|qid| contract_ids.contains(qid.as_str()))
            })
            .collect();
        qualifying.sort_by(|(t_a, a), (t_b, b)| {
            t_a.cmp(t_b)
                .then_with(|| a.available_utc.cmp(&b.available_utc))
                .then_with(|| a.who_or_outlet.cmp(&b.who_or_outlet))
        });

        let first_public = qualifying.first().map(|(_, item)| (*item).clone());
        let second_source = qualifying.get(1).map(|(_, item)| (*item).clone());
        let first_pub_ms = qualifying.first().map(|(t, _)| *t);

        let mut earlier_signals: Vec<AuditEvidenceItem> = parsed_items
            .iter()
            .copied()
            .filter(|(pub_ms, item)| {
                *pub_ms <= cutoff
                    && !item.satisfies_rules
                    && first_pub_ms.is_none_or(|fp| *pub_ms <= fp)
            })
            .map(|(_, item)| item.clone())
            .collect();
        earlier_signals.sort_by(|a, b| a.published_utc.cmp(&b.published_utc));

        let per_question_resolution = question_contracts
            .iter()
            .map(|contract| {
                let earliest_for_q = qualifying
                    .iter()
                    .find(|(_, item)| {
                        item.qualified_question_ids
                            .iter()
                            .any(|qid| qid == &contract.question_id)
                    })
                    .map(|(_, item)| *item);
                match earliest_for_q {
                    Some(item) => PerQuestionResolution {
                        question_id: contract.question_id.clone(),
                        resolution_status: ResolutionStatusAtCutoff::Resolved,
                        resolved_published_utc: Some(item.published_utc.clone()),
                        adjudication_ev_handle: item.ev_handle.clone(),
                    },
                    None => PerQuestionResolution {
                        question_id: contract.question_id.clone(),
                        resolution_status: ResolutionStatusAtCutoff::Unresolved,
                        resolved_published_utc: None,
                        adjudication_ev_handle: None,
                    },
                }
            })
            .collect();

        Ok(BiTemporalAuditOutcome {
            as_of_utc: as_of_utc.to_owned(),
            first_public,
            second_source,
            earlier_signals,
            per_question_resolution,
        })
    }

    /// Expands a minute-precision (or finer) UTC timestamp into its enclosing
    /// `[HH:MM:00.000Z, HH:MM:59.999Z]` join window (`§9.2`, `GATE-08`).
    pub fn minute_join_window(published_utc: &str) -> Result<MinuteWindow, VerifyError> {
        let ms = parse_utc_millis(published_utc)?;
        let minute_floor = ms.0.div_euclid(60_000) * 60_000;
        Ok(MinuteWindow {
            start_ms: UtcMillis(minute_floor),
            end_ms: UtcMillis(minute_floor + 59_999),
        })
    }

    /// Joins a list of `(label, timestamp_utc)` ticks against `published_utc`'s
    /// `[HH:MM:00.000Z, HH:MM:59.999Z]` window, returning matching ticks in chronological order.
    pub fn join_within_minute_window<'a>(
        published_utc: &str,
        ticks: &'a [(&'a str, &'a str)],
    ) -> Result<Vec<(&'a str, &'a str)>, VerifyError> {
        let window = Self::minute_join_window(published_utc)?;
        let mut matched: Vec<(UtcMillis, (&'a str, &'a str))> = Vec::new();
        for &(label, ts_str) in ticks {
            let ts = parse_utc_millis(ts_str)?;
            if window.contains(ts) {
                matched.push((ts, (label, ts_str)));
            }
        }
        matched.sort_by_key(|(ts, _)| *ts);
        Ok(matched.into_iter().map(|(_, pair)| pair).collect())
    }
}

/// Parses UTC timestamps in `YYYY-MM-DD HH:MM`, `YYYY-MM-DDTHH:MMZ`,
/// `YYYY-MM-DDTHH:MM:SSZ`, or `YYYY-MM-DDTHH:MM:SS.mmmZ` (space or `T` separator)
/// via canonical [`fcast_core::time::parse_utc_epoch_ms`].
pub fn parse_utc_millis(raw: &str) -> Result<UtcMillis, VerifyError> {
    fcast_core::time::parse_utc_epoch_ms(raw)
        .map(UtcMillis)
        .map_err(|_| VerifyError::InvalidTimestamp(raw.to_owned()))
}

// ---------------------------------------------------------------------------
// ValueExtractionLadder & THIN_PUBLISH_FLOOR (§11.2)
// ---------------------------------------------------------------------------

/// Which rung of the 4-rung `ValueExtractionLadder` succeeded (`§11.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionRung {
    Rung1StrictJson,
    Rung2FencedBlock,
    Rung3AnchorScan,
    Rung4ParserPass,
}

/// Result of extracting a binary probability through `ValueExtractionLadder`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExtractedProbability {
    pub probability: f64,
    pub rung: ExtractionRung,
}

/// Result of extracting a multiple-choice probability map through `ValueExtractionLadder`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedOptionMap {
    pub probabilities: BTreeMap<String, f64>,
    pub rung: ExtractionRung,
}

/// Applies `THIN_PUBLISH_FLOOR` (`[0.05, 0.95]` when `survivors <= 1`, else `[0.02, 0.98]`, `§11.2`).
#[must_use]
pub fn apply_publish_floor(p: f64, survivors: usize) -> f64 {
    let (lo, hi) = if survivors <= 1 {
        THIN_PUBLISH_FLOOR
    } else {
        NORMAL_PUBLISH_BOUNDS
    };
    let safe = if p.is_finite() { p } else { 0.5 };
    safe.clamp(lo, hi)
}

/// 4-rung fidelity-checked deterministic extractor (`§11.2`).
///
/// Rejects duplicate JSON keys, boolean-as-number coercions, truncated numeric literals,
/// and missing/unknown option multisets.
#[derive(Debug, Default, Clone, Copy)]
pub struct ValueExtractionLadder;

impl ValueExtractionLadder {
    /// Extracts a binary `"probability"` in `[0.0, 1.0]` across up to 4 rungs.
    ///
    /// `parser_pass` (Rung 4) is invoked only when Rungs 1..3 fail on non-fatal formatting
    /// (never to bypass a hard integrity error like duplicate JSON keys or boolean coercion).
    pub fn extract_binary_probability<F>(
        raw: &str,
        parser_pass: Option<F>,
    ) -> Result<ExtractedProbability, VerifyError>
    where
        F: FnOnce(&str) -> Result<String, String>,
    {
        let trimmed = raw.trim();

        // Rung 1: Strict JSON schema on the full payload.
        if trimmed.starts_with('{') {
            return parse_strict_prob_json(trimmed).map(|probability| ExtractedProbability {
                probability,
                rung: ExtractionRung::Rung1StrictJson,
            });
        }

        // Rung 2: Fenced markdown block extraction (```json ... ``` or ``` ... ```).
        if let Some(block) = extract_fenced_block(trimmed) {
            return parse_strict_prob_json(block).map(|probability| ExtractedProbability {
                probability,
                rung: ExtractionRung::Rung2FencedBlock,
            });
        }

        // Rung 3: Deterministic anchor extraction (`probability: <num>` or embedded JSON object).
        if let Some(res) = extract_anchor_probability(trimmed) {
            let probability = res?;
            return Ok(ExtractedProbability {
                probability,
                rung: ExtractionRung::Rung3AnchorScan,
            });
        }

        // Rung 4: Dedicated parser pass at `parser.temperature = 0`.
        if let Some(repair_fn) = parser_pass {
            let repaired = repair_fn(trimmed)
                .map_err(|msg| VerifyError::ExtractionExhausted(format!("rung4: {msg}")))?;
            let probability = parse_strict_prob_json(repaired.trim())?;
            return Ok(ExtractedProbability {
                probability,
                rung: ExtractionRung::Rung4ParserPass,
            });
        }

        Err(VerifyError::ExtractionExhausted(
            "no valid probability found in rungs 1..3 and no rung-4 parser supplied".to_owned(),
        ))
    }

    /// Extracts and validates a multiple-choice probability map against `canonical_options`,
    /// rejecting duplicate keys, booleans, truncated literals, and missing/unknown options.
    pub fn extract_option_multiset<F>(
        raw: &str,
        canonical_options: &[&str],
        parser_pass: Option<F>,
    ) -> Result<ExtractedOptionMap, VerifyError>
    where
        F: FnOnce(&str) -> Result<String, String>,
    {
        let trimmed = raw.trim();

        if trimmed.starts_with('{') {
            let probabilities = parse_strict_option_map_json(trimmed, canonical_options)?;
            return Ok(ExtractedOptionMap {
                probabilities,
                rung: ExtractionRung::Rung1StrictJson,
            });
        }

        if let Some(block) = extract_fenced_block(trimmed) {
            let probabilities = parse_strict_option_map_json(block, canonical_options)?;
            return Ok(ExtractedOptionMap {
                probabilities,
                rung: ExtractionRung::Rung2FencedBlock,
            });
        }

        if let Some(obj_slice) = extract_first_json_object(trimmed) {
            let probabilities = parse_strict_option_map_json(obj_slice, canonical_options)?;
            return Ok(ExtractedOptionMap {
                probabilities,
                rung: ExtractionRung::Rung3AnchorScan,
            });
        }

        if let Some(repair_fn) = parser_pass {
            let repaired = repair_fn(trimmed)
                .map_err(|msg| VerifyError::ExtractionExhausted(format!("rung4: {msg}")))?;
            let probabilities = parse_strict_option_map_json(repaired.trim(), canonical_options)?;
            return Ok(ExtractedOptionMap {
                probabilities,
                rung: ExtractionRung::Rung4ParserPass,
            });
        }

        Err(VerifyError::ExtractionExhausted(
            "no valid option map found in rungs 1..3 and no rung-4 parser supplied".to_owned(),
        ))
    }
}

/// Strict Rung-1 JSON parser for `{"probability": <float in [0, 1]>}` with duplicate-key rejection.
pub fn parse_strict_prob_json(raw: &str) -> Result<f64, VerifyError> {
    check_no_duplicate_keys_and_complete(raw)?;
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| VerifyError::TruncatedOrInvalidLiteral(e.to_string()))?;
    let obj = value.as_object().ok_or_else(|| {
        VerifyError::TruncatedOrInvalidLiteral("expected top-level JSON object".to_owned())
    })?;
    let prob_val = obj.get("probability").ok_or_else(|| {
        VerifyError::TruncatedOrInvalidLiteral("missing 'probability' field".to_owned())
    })?;
    strict_prob_value("probability", prob_val)
}

fn parse_strict_option_map_json(
    raw: &str,
    canonical_options: &[&str],
) -> Result<BTreeMap<String, f64>, VerifyError> {
    check_no_duplicate_keys_and_complete(raw)?;
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| VerifyError::TruncatedOrInvalidLiteral(e.to_string()))?;
    let obj = value.as_object().ok_or_else(|| {
        VerifyError::TruncatedOrInvalidLiteral("expected top-level JSON object".to_owned())
    })?;
    let map_obj = obj
        .get("options")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(obj);

    let mut canon_index: BTreeMap<String, &str> = BTreeMap::new();
    for &opt in canonical_options {
        canon_index.insert(normalize_option_label(opt), opt);
    }

    let mut restored = BTreeMap::new();
    for (k, v) in map_obj {
        let norm_k = normalize_option_label(k);
        let Some(&canon) = canon_index.get(&norm_k) else {
            return Err(VerifyError::OptionMultisetMismatch(format!(
                "unknown option label {k:?}"
            )));
        };
        let p = strict_prob_value(canon, v)?;
        if restored.insert(canon.to_owned(), p).is_some() {
            return Err(VerifyError::DuplicateJsonKey(canon.to_owned()));
        }
    }

    for &opt in canonical_options {
        if !restored.contains_key(opt) {
            return Err(VerifyError::OptionMultisetMismatch(format!(
                "missing canonical option {opt:?}"
            )));
        }
    }

    Ok(restored)
}

fn normalize_option_label(s: &str) -> String {
    s.replace(['\u{2013}', '\u{2014}'], "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn strict_prob_value(field: &str, val: &serde_json::Value) -> Result<f64, VerifyError> {
    if val.is_boolean() {
        return Err(VerifyError::BooleanAsNumber(field.to_owned()));
    }
    let Some(num) = val.as_f64() else {
        return Err(VerifyError::TruncatedOrInvalidLiteral(format!(
            "field {field} is not a numeric literal"
        )));
    };
    if !num.is_finite() || !(0.0..=1.0).contains(&num) {
        return Err(VerifyError::ProbabilityOutOfRange(num));
    }
    Ok(num)
}

fn extract_fenced_block(text: &str) -> Option<&str> {
    let start_fence = text.find("```")?;
    let after_open = &text[start_fence + 3..];
    let newline_idx = after_open.find('\n')?;
    let body = &after_open[newline_idx + 1..];
    let end_fence = body.find("```")?;
    Some(body[..end_fence].trim())
}

fn extract_first_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let mut depth = 0_usize;
    let mut in_str = false;
    let mut escaped = false;
    for (idx, &b) in bytes[start..].iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&text[start..=start + idx]);
                }
            }
            _ => {}
        }
    }
    // Truncated object: return slice so Rung 1 strict parser reports TruncatedOrInvalidLiteral.
    Some(&text[start..])
}

fn extract_anchor_probability(text: &str) -> Option<Result<f64, VerifyError>> {
    if let Some(obj) = extract_first_json_object(text) {
        return Some(parse_strict_prob_json(obj));
    }

    // Look for `probability:` or `probability =` anchor in prose.
    let lower = text.to_ascii_lowercase();
    let mut hits: Vec<Result<f64, VerifyError>> = Vec::new();
    let mut search_from = 0_usize;
    while let Some(rel) = lower[search_from..].find("probability") {
        let pos = search_from + rel + "probability".len();
        search_from = pos;
        let rest = text[pos..].trim_start();
        let Some(after_sep) = rest
            .strip_prefix(':')
            .or_else(|| rest.strip_prefix('='))
            .map(str::trim_start)
        else {
            continue;
        };
        let token = after_sep
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ')' | ']'))
            .next()
            .unwrap_or("");
        if token.is_empty() {
            hits.push(Err(VerifyError::TruncatedOrInvalidLiteral(
                "empty probability anchor".to_owned(),
            )));
            continue;
        }
        if token.eq_ignore_ascii_case("true") || token.eq_ignore_ascii_case("false") {
            hits.push(Err(VerifyError::BooleanAsNumber("probability".to_owned())));
            continue;
        }
        let (num_str, is_pct) = match token.strip_suffix('%') {
            Some(s) => (s, true),
            None => (token, false),
        };
        if num_str.ends_with('.') || num_str.ends_with('e') || num_str.ends_with('E') {
            hits.push(Err(VerifyError::TruncatedOrInvalidLiteral(
                token.to_owned(),
            )));
            continue;
        }
        match num_str.parse::<f64>() {
            Ok(v) => {
                let p = if is_pct { v / 100.0 } else { v };
                if !p.is_finite() || !(0.0..=1.0).contains(&p) {
                    hits.push(Err(VerifyError::ProbabilityOutOfRange(p)));
                } else {
                    hits.push(Ok(p));
                }
            }
            Err(_) => hits.push(Err(VerifyError::TruncatedOrInvalidLiteral(
                token.to_owned(),
            ))),
        }
    }

    match hits.len() {
        0 => None,
        1 => hits.into_iter().next(),
        _ => Some(Err(VerifyError::DuplicateJsonKey("probability".to_owned()))),
    }
}

/// Lightweight deterministic JSON structural scanner that rejects duplicate keys at every object
/// level, enforces a nesting depth cap (`MAX_JSON_DEPTH = 64`), preserves full UTF-8 strings, and
/// catches truncated numeric literals before `serde_json` overwrites duplicate keys.
fn check_no_duplicate_keys_and_complete(raw: &str) -> Result<(), VerifyError> {
    const MAX_JSON_DEPTH: usize = 64;
    let bytes = raw.as_bytes();
    let mut i = 0_usize;

    fn skip_ws(b: &[u8], i: &mut usize) {
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
    }

    fn parse_string(raw: &str, b: &[u8], i: &mut usize) -> Result<String, VerifyError> {
        if *i >= b.len() || b[*i] != b'"' {
            return Err(VerifyError::TruncatedOrInvalidLiteral(
                "expected '\"'".to_owned(),
            ));
        }
        *i += 1;
        let mut out = String::new();
        let mut seg_start = *i;
        while *i < b.len() {
            let c = b[*i];
            if c == b'"' {
                out.push_str(&raw[seg_start..*i]);
                *i += 1;
                return Ok(out);
            }
            if c == b'\\' {
                out.push_str(&raw[seg_start..*i]);
                *i += 1;
                if *i >= b.len() {
                    return Err(VerifyError::TruncatedOrInvalidLiteral(
                        "truncated string escape".to_owned(),
                    ));
                }
                let esc = b[*i];
                *i += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'u' => {
                        if *i + 4 > b.len() {
                            return Err(VerifyError::TruncatedOrInvalidLiteral(
                                "truncated unicode escape".to_owned(),
                            ));
                        }
                        let hex = std::str::from_utf8(&b[*i..*i + 4]).map_err(|_| {
                            VerifyError::TruncatedOrInvalidLiteral("bad utf8".into())
                        })?;
                        let code = u32::from_str_radix(hex, 16).map_err(|_| {
                            VerifyError::TruncatedOrInvalidLiteral("bad hex".into())
                        })?;
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                        *i += 4;
                    }
                    _ => {
                        return Err(VerifyError::TruncatedOrInvalidLiteral(
                            "invalid escape".to_owned(),
                        ));
                    }
                }
                seg_start = *i;
            } else {
                *i += 1;
            }
        }
        Err(VerifyError::TruncatedOrInvalidLiteral(
            "unterminated string".to_owned(),
        ))
    }

    fn parse_value(raw: &str, b: &[u8], i: &mut usize, depth: usize) -> Result<(), VerifyError> {
        if depth > MAX_JSON_DEPTH {
            return Err(VerifyError::TruncatedOrInvalidLiteral(
                "JSON nesting depth limit exceeded".to_owned(),
            ));
        }
        skip_ws(b, i);
        if *i >= b.len() {
            return Err(VerifyError::TruncatedOrInvalidLiteral(
                "unexpected EOF".to_owned(),
            ));
        }
        match b[*i] {
            b'{' => {
                *i += 1;
                skip_ws(b, i);
                let mut seen = BTreeSet::new();
                if *i < b.len() && b[*i] == b'}' {
                    *i += 1;
                    return Ok(());
                }
                loop {
                    skip_ws(b, i);
                    let key = parse_string(raw, b, i)?;
                    if !seen.insert(key.clone()) {
                        return Err(VerifyError::DuplicateJsonKey(key));
                    }
                    skip_ws(b, i);
                    if *i >= b.len() || b[*i] != b':' {
                        return Err(VerifyError::TruncatedOrInvalidLiteral(
                            "expected ':' after key".to_owned(),
                        ));
                    }
                    *i += 1;
                    parse_value(raw, b, i, depth + 1)?;
                    skip_ws(b, i);
                    if *i >= b.len() {
                        return Err(VerifyError::TruncatedOrInvalidLiteral(
                            "unterminated object".to_owned(),
                        ));
                    }
                    match b[*i] {
                        b',' => *i += 1,
                        b'}' => {
                            *i += 1;
                            return Ok(());
                        }
                        _ => {
                            return Err(VerifyError::TruncatedOrInvalidLiteral(
                                "expected ',' or '}'".to_owned(),
                            ));
                        }
                    }
                }
            }
            b'[' => {
                *i += 1;
                skip_ws(b, i);
                if *i < b.len() && b[*i] == b']' {
                    *i += 1;
                    return Ok(());
                }
                loop {
                    parse_value(raw, b, i, depth + 1)?;
                    skip_ws(b, i);
                    if *i >= b.len() {
                        return Err(VerifyError::TruncatedOrInvalidLiteral(
                            "unterminated array".to_owned(),
                        ));
                    }
                    match b[*i] {
                        b',' => *i += 1,
                        b']' => {
                            *i += 1;
                            return Ok(());
                        }
                        _ => {
                            return Err(VerifyError::TruncatedOrInvalidLiteral(
                                "expected ',' or ']'".to_owned(),
                            ));
                        }
                    }
                }
            }
            b'"' => {
                let _ = parse_string(raw, b, i)?;
                Ok(())
            }
            b't' => {
                if b[*i..].starts_with(b"true") {
                    *i += 4;
                    Ok(())
                } else {
                    Err(VerifyError::TruncatedOrInvalidLiteral(
                        "truncated true".to_owned(),
                    ))
                }
            }
            b'f' => {
                if b[*i..].starts_with(b"false") {
                    *i += 5;
                    Ok(())
                } else {
                    Err(VerifyError::TruncatedOrInvalidLiteral(
                        "truncated false".to_owned(),
                    ))
                }
            }
            b'n' => {
                if b[*i..].starts_with(b"null") {
                    *i += 4;
                    Ok(())
                } else {
                    Err(VerifyError::TruncatedOrInvalidLiteral(
                        "truncated null".to_owned(),
                    ))
                }
            }
            b'-' | b'0'..=b'9' => {
                let start = *i;
                if b[*i] == b'-' {
                    *i += 1;
                }
                let digits_start = *i;
                while *i < b.len() && b[*i].is_ascii_digit() {
                    *i += 1;
                }
                if *i == digits_start {
                    return Err(VerifyError::TruncatedOrInvalidLiteral(
                        "truncated negative literal".to_owned(),
                    ));
                }
                if *i < b.len() && b[*i] == b'.' {
                    *i += 1;
                    let frac_start = *i;
                    while *i < b.len() && b[*i].is_ascii_digit() {
                        *i += 1;
                    }
                    if *i == frac_start {
                        return Err(VerifyError::TruncatedOrInvalidLiteral(
                            "truncated decimal fraction".to_owned(),
                        ));
                    }
                }
                if *i < b.len() && matches!(b[*i], b'e' | b'E') {
                    *i += 1;
                    if *i < b.len() && matches!(b[*i], b'+' | b'-') {
                        *i += 1;
                    }
                    let exp_start = *i;
                    while *i < b.len() && b[*i].is_ascii_digit() {
                        *i += 1;
                    }
                    if *i == exp_start {
                        return Err(VerifyError::TruncatedOrInvalidLiteral(
                            "truncated exponent".to_owned(),
                        ));
                    }
                }
                if *i == start {
                    return Err(VerifyError::TruncatedOrInvalidLiteral(
                        "empty number".to_owned(),
                    ));
                }
                Ok(())
            }
            _ => Err(VerifyError::TruncatedOrInvalidLiteral(
                "unexpected token".to_owned(),
            )),
        }
    }

    parse_value(raw, bytes, &mut i, 0)?;
    skip_ws(bytes, &mut i);
    if i != bytes.len() {
        return Err(VerifyError::TruncatedOrInvalidLiteral(
            "trailing tokens after JSON value".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_scanner_guards_depth_and_preserves_utf8_keys() {
        // Reject deeply nested JSON (> MAX_JSON_DEPTH = 64) without stack overflow
        let deep = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(matches!(
            parse_strict_prob_json(&deep),
            Err(VerifyError::TruncatedOrInvalidLiteral(_))
        ));

        // Preserve UTF-8 multi-byte keys accurately and reject Unicode duplicate keys
        let dup_utf8 = "{\"α\": 0.5, \"α\": 0.6}";
        assert!(matches!(
            parse_strict_prob_json(dup_utf8),
            Err(VerifyError::DuplicateJsonKey(k)) if k == "α"
        ));

        let ok_utf8 = "{\"note\": \"αβγ\", \"probability\": 0.42}";
        let p = parse_strict_prob_json(ok_utf8).expect("valid utf8 json");
        assert!((p - 0.42).abs() < 1e-12);
    }
}
