//! Pure-Rust Metaculus client model (`§5.2`, `§11.4`, `§12.5`, `GATE-11`),
//! Mantic Crucible wire adapter, and ForecastBench 3-tier crowd prior backfill.

use std::collections::{BTreeMap, HashMap};

use fcast_core::coherence::{CoherenceError, build_metaculus_cdf};
use fcast_core::forecast::ProbClamped;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Canonical 13-percentile anchor grid (`§11.4`).
pub const CANONICAL_PERCENTILES_13: [f64; 13] = [
    1.0, 2.5, 5.0, 10.0, 20.0, 40.0, 50.0, 60.0, 80.0, 90.0, 95.0, 97.5, 99.0,
];
/// Metaculus API minimum adjacent CDF step (`5e-5` `[UPSTREAM-SPEC]`).
pub const METACULUS_CDF_MIN_STEP: f64 = 5e-5;
/// Metaculus API maximum adjacent CDF step (`0.59` `[UPSTREAM-SPEC]`).
pub const METACULUS_CDF_MAX_STEP: f64 = 0.59;
/// Default fixed-point simplex floor for `MultipleChoiceQuestion` (`0.01` `[BUDGET]`).
pub const DEFAULT_MC_SIMPLEX_FLOOR: f64 = 0.01;
/// Maximum number of PMF bins accepted by Mantic Crucible (`31` `[UPSTREAM-SPEC]`).
pub const MANTIC_MAX_BINS: usize = 31;
/// Minimum open-tail probability mass on Mantic open-bounded quantitative contracts (`0.05` `[BUDGET]`).
pub const MANTIC_OPEN_TAIL_FLOOR: f64 = 0.05;

#[derive(Debug, Error, PartialEq)]
pub enum MetaculusError {
    #[error("coherence/cdf error: {0}")]
    Coherence(#[from] CoherenceError),
    #[error("invalid simplex input: {0}")]
    InvalidSimplex(&'static str),
    #[error("unknown option label: {0}")]
    UnknownOptionLabel(String),
    #[error("missing canonical option(s): {0:?}")]
    MissingCanonicalOptions(Vec<String>),
    #[error("invalid mantic payload: {0}")]
    InvalidManticPayload(String),
    #[error("invalid mantic pmf: {0}")]
    InvalidManticPmf(&'static str),
}

/// Shared metadata carried by Metaculus question variants.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QuestionMeta {
    pub id: u64,
    pub title: String,
    pub resolution_criteria: String,
    pub open_time_utc: Option<String>,
    pub scheduled_close_time_utc: Option<String>,
    pub scheduled_resolve_time_utc: Option<String>,
    pub spot_scoring_time_utc: Option<String>,
}

/// Binary Yes/No Metaculus question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryQuestion {
    pub meta: QuestionMeta,
}

/// Multiple-choice Metaculus question with canonical option labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultipleChoiceQuestion {
    pub meta: QuestionMeta,
    pub options: Vec<String>,
}

impl MultipleChoiceQuestion {
    /// Restore LLM-mangled option labels (en/em dashes, curly quotes, whitespace, case)
    /// to canonical labels and apply fixed-point iterative simplex flooring (`>= floor`).
    pub fn normalize_and_restore(
        &self,
        predicted: &BTreeMap<String, f64>,
        floor: f64,
    ) -> Result<BTreeMap<String, f64>, MetaculusError> {
        let restored = restore_option_names(predicted, &self.options)?;
        let raw_probs: Vec<f64> = self
            .options
            .iter()
            .map(|opt| restored.get(opt).copied().unwrap_or(0.0))
            .collect();
        let floored = fixed_point_simplex_floor(&raw_probs, floor)?;
        let mut out = BTreeMap::new();
        for (opt, p) in self.options.iter().zip(floored) {
            out.insert(opt.clone(), p);
        }
        Ok(out)
    }
}

/// Continuous numeric Metaculus question (`201`-point CDF, linear or `zero_point` log-scale).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumericQuestion {
    pub meta: QuestionMeta,
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub open_lower_bound: bool,
    pub open_upper_bound: bool,
    pub zero_point: Option<f64>,
    pub unit: Option<String>,
}

impl NumericQuestion {
    /// Build a `201`-point API-compliant Metaculus CDF from `(value, prob)` percentile anchors.
    pub fn build_cdf(&self, percentiles: &[(f64, f64)]) -> Result<Vec<f64>, MetaculusError> {
        Ok(build_metaculus_cdf(
            percentiles,
            self.lower_bound,
            self.upper_bound,
            self.open_lower_bound,
            self.open_upper_bound,
            self.zero_point,
            None,
            METACULUS_CDF_MIN_STEP,
            METACULUS_CDF_MAX_STEP,
        )?)
    }
}

/// Discrete Metaculus question (`inbound_outcome_count + 1`-point CDF).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscreteQuestion {
    pub meta: QuestionMeta,
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub open_lower_bound: bool,
    pub open_upper_bound: bool,
    pub inbound_outcome_count: usize,
}

impl DiscreteQuestion {
    /// Build an `inbound_outcome_count + 1`-point API-compliant discrete CDF.
    pub fn build_cdf(&self, percentiles: &[(f64, f64)]) -> Result<Vec<f64>, MetaculusError> {
        Ok(build_metaculus_cdf(
            percentiles,
            self.lower_bound,
            self.upper_bound,
            self.open_lower_bound,
            self.open_upper_bound,
            None,
            Some(self.inbound_outcome_count),
            METACULUS_CDF_MIN_STEP,
            METACULUS_CDF_MAX_STEP,
        )?)
    }
}

/// Date Metaculus question (UNIX epoch seconds mapped onto a continuous `201`-point CDF).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DateQuestion {
    pub meta: QuestionMeta,
    pub lower_bound_epoch_s: f64,
    pub upper_bound_epoch_s: f64,
    pub open_lower_bound: bool,
    pub open_upper_bound: bool,
}

impl DateQuestion {
    /// Build a `201`-point date CDF with open-bound tail compression (`§11.4`) so far-tail
    /// dates stay smooth inside `[0.001, 0.999]`.
    pub fn build_cdf(&self, epoch_percentiles: &[(f64, f64)]) -> Result<Vec<f64>, MetaculusError> {
        let span = self.upper_bound_epoch_s - self.lower_bound_epoch_s;
        if !span.is_finite() || span <= 0.0 {
            return Err(MetaculusError::Coherence(CoherenceError::InvalidBounds(
                self.lower_bound_epoch_s,
                self.upper_bound_epoch_s,
            )));
        }

        // Open-bound date compression: map out-of-range tail timestamps smoothly toward the
        // boundary via hyperbolic compression while preserving probability anchors.
        let compressed: Vec<(f64, f64)> = epoch_percentiles
            .iter()
            .map(|&(ts, p)| {
                let in_open_tail = (self.open_lower_bound && ts <= self.lower_bound_epoch_s)
                    || (self.open_upper_bound && ts >= self.upper_bound_epoch_s);
                let adj_p = if in_open_tail {
                    p.clamp(0.001, 0.999)
                } else {
                    p
                };
                let adj_ts = if self.open_lower_bound && ts < self.lower_bound_epoch_s {
                    let excess = (self.lower_bound_epoch_s - ts) / span;
                    self.lower_bound_epoch_s - span * (excess / (1.0 + excess)) * 0.05
                } else if self.open_upper_bound && ts > self.upper_bound_epoch_s {
                    let excess = (ts - self.upper_bound_epoch_s) / span;
                    self.upper_bound_epoch_s + span * (excess / (1.0 + excess)) * 0.05
                } else {
                    ts
                };
                (adj_ts, adj_p)
            })
            .collect();

        Ok(build_metaculus_cdf(
            &compressed,
            self.lower_bound_epoch_s,
            self.upper_bound_epoch_s,
            self.open_lower_bound,
            self.open_upper_bound,
            None,
            None,
            METACULUS_CDF_MIN_STEP,
            METACULUS_CDF_MAX_STEP,
        )?)
    }
}

/// Conditional pair of Metaculus questions (`if condition = Yes` vs `if condition = No`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalQuestion {
    pub meta: QuestionMeta,
    pub condition_id: u64,
    pub condition_child_id: u64,
    pub question_yes: Box<MetaculusQuestion>,
    pub question_no: Box<MetaculusQuestion>,
}

/// Group of related Metaculus subquestions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupOfQuestions {
    pub meta: QuestionMeta,
    pub subquestions: Vec<MetaculusQuestion>,
    pub exhaustive: bool,
}

/// Full typed Metaculus question hierarchy (`§12.5`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MetaculusQuestion {
    Binary(BinaryQuestion),
    MultipleChoice(MultipleChoiceQuestion),
    Numeric(NumericQuestion),
    Discrete(DiscreteQuestion),
    Date(DateQuestion),
    Conditional(ConditionalQuestion),
    Group(GroupOfQuestions),
}

impl MetaculusQuestion {
    #[must_use]
    pub fn meta(&self) -> &QuestionMeta {
        match self {
            Self::Binary(q) => &q.meta,
            Self::MultipleChoice(q) => &q.meta,
            Self::Numeric(q) => &q.meta,
            Self::Discrete(q) => &q.meta,
            Self::Date(q) => &q.meta,
            Self::Conditional(q) => &q.meta,
            Self::Group(q) => &q.meta,
        }
    }
}

/// Fixed-point iterative simplex flooring (`§11.4`).
///
/// Guarantees every probability satisfies `p_k >= floor` AND `sum(p_k) == 1.0`
/// (avoiding naive clip-then-renormalize floor violations).
pub fn fixed_point_simplex_floor(probs: &[f64], floor: f64) -> Result<Vec<f64>, MetaculusError> {
    let n = probs.len();
    if n < 2 {
        return Err(MetaculusError::InvalidSimplex("need at least 2 options"));
    }
    if !floor.is_finite() || floor < 0.0 || floor * (n as f64) >= 1.0 {
        return Err(MetaculusError::InvalidSimplex(
            "floor must be non-negative and floor * n < 1.0",
        ));
    }
    if probs.iter().any(|p| !p.is_finite()) {
        return Err(MetaculusError::InvalidSimplex("non-finite probability"));
    }

    let mut p: Vec<f64> = probs.iter().map(|&x| x.max(0.0)).collect();
    let s: f64 = p.iter().sum();
    if s > 0.0 {
        for x in &mut p {
            *x /= s;
        }
    } else {
        p.fill(1.0 / (n as f64));
    }

    let mut fixed = vec![false; n];
    for _ in 0..32 {
        let mut any_new = false;
        for i in 0..n {
            if !fixed[i] && p[i] < floor - 1e-15 {
                fixed[i] = true;
                any_new = true;
            }
        }
        if !any_new {
            break;
        }
        let fixed_count = fixed.iter().filter(|&&f| f).count();
        let rem_mass = 1.0 - floor * (fixed_count as f64);
        let free_sum: f64 = p
            .iter()
            .enumerate()
            .filter(|&(i, _)| !fixed[i])
            .map(|(_, &v)| v)
            .sum();
        let free_count = n - fixed_count;
        for i in 0..n {
            if fixed[i] {
                p[i] = floor;
            } else if free_sum > 0.0 {
                p[i] = (p[i] / free_sum) * rem_mass;
            } else {
                p[i] = rem_mass / (free_count as f64);
            }
        }
    }
    Ok(p)
}

fn normalize_option_label(raw: &str) -> String {
    let replaced = raw
        .replace(['\u{2013}', '\u{2014}', '\u{2212}'], "-")
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201C}', '\u{201D}'], "\"");
    replaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Repair LLM-mangled option labels (en-dashes, em-dashes, curly quotes, extra whitespace,
/// casing) back onto the exact canonical Metaculus option labels (`§11.4`).
pub fn restore_option_names(
    predicted: &BTreeMap<String, f64>,
    canonical_options: &[String],
) -> Result<BTreeMap<String, f64>, MetaculusError> {
    let mut canon_idx: HashMap<String, &String> = HashMap::with_capacity(canonical_options.len());
    for opt in canonical_options {
        canon_idx.insert(normalize_option_label(opt), opt);
    }

    let mut out = BTreeMap::new();
    for (k, &v) in predicted {
        let nk = normalize_option_label(k);
        let Some(&canonical) = canon_idx.get(&nk) else {
            return Err(MetaculusError::UnknownOptionLabel(k.clone()));
        };
        out.insert(canonical.clone(), v);
    }

    let missing: Vec<String> = canonical_options
        .iter()
        .filter(|opt| !out.contains_key(*opt))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(MetaculusError::MissingCanonicalOptions(missing));
    }
    Ok(out)
}

/// Metaculus tournament `bot_leaderboard_status` field (`§5.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotLeaderboardStatus {
    BotsOnly,
    Include,
    Exclude,
    Unspecified,
}

impl BotLeaderboardStatus {
    /// Returns `true` for prize-eligible automated bot tracks (`bots_only | include`).
    #[must_use]
    pub const fn is_prize_eligible(self) -> bool {
        matches!(self, Self::BotsOnly | Self::Include)
    }

    #[must_use]
    pub const fn as_api_str(self) -> &'static str {
        match self {
            Self::BotsOnly => "bots_only",
            Self::Include => "include",
            Self::Exclude => "exclude",
            Self::Unspecified => "unspecified",
        }
    }
}

/// Paginated `GET /api/posts/` filter with `bot_leaderboard_status` auto-discovery (`§5.2`, `§12.5`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApiFilter {
    pub tournaments: Vec<String>,
    pub statuses: Vec<String>,
    pub allowed_types: Vec<String>,
    pub bot_leaderboard_statuses: Vec<BotLeaderboardStatus>,
    pub scheduled_resolve_time_gt: Option<String>,
    pub scheduled_resolve_time_lt: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl ApiFilter {
    /// Build a filter that auto-discovers open prize-eligible tournaments via
    /// `bot_leaderboard_status ∈ {bots_only, include}` without hardcoding tournament IDs.
    #[must_use]
    pub fn auto_discover_prize_tournaments() -> Self {
        Self {
            tournaments: Vec::new(),
            statuses: vec!["open".to_string()],
            allowed_types: vec![
                "binary".to_string(),
                "multiple_choice".to_string(),
                "numeric".to_string(),
                "discrete".to_string(),
                "date".to_string(),
            ],
            bot_leaderboard_statuses: vec![
                BotLeaderboardStatus::BotsOnly,
                BotLeaderboardStatus::Include,
            ],
            scheduled_resolve_time_gt: None,
            scheduled_resolve_time_lt: None,
            limit: Some(100),
            offset: Some(0),
        }
    }

    /// Check whether a tournament matches this filter's open & bot-leaderboard criteria.
    #[must_use]
    pub fn matches_tournament(&self, status: BotLeaderboardStatus, is_open: bool) -> bool {
        let status_ok =
            self.statuses.is_empty() || (is_open && self.statuses.iter().any(|s| s == "open"));
        let lb_ok = self.bot_leaderboard_statuses.is_empty()
            || self.bot_leaderboard_statuses.contains(&status);
        status_ok && lb_ok
    }

    /// Serialize filter fields into deterministic query key/value pairs.
    #[must_use]
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();
        for t in &self.tournaments {
            params.push(("tournaments".to_string(), t.clone()));
        }
        for s in &self.statuses {
            params.push(("statuses".to_string(), s.clone()));
        }
        for ty in &self.allowed_types {
            params.push(("forecast_type".to_string(), ty.clone()));
        }
        for bls in &self.bot_leaderboard_statuses {
            params.push((
                "bot_leaderboard_status".to_string(),
                bls.as_api_str().to_string(),
            ));
        }
        if let Some(gt) = &self.scheduled_resolve_time_gt {
            params.push(("scheduled_resolve_time__gt".to_string(), gt.clone()));
        }
        if let Some(lt) = &self.scheduled_resolve_time_lt {
            params.push(("scheduled_resolve_time__lt".to_string(), lt.clone()));
        }
        if let Some(limit) = self.limit {
            params.push(("limit".to_string(), limit.to_string()));
        }
        if let Some(offset) = self.offset {
            params.push(("offset".to_string(), offset.to_string()));
        }
        params
    }
}

/// First-class adapter for Mantic Crucible (`competitions.mantic.com`, `§5.2`, `§11.4`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ManticCrucibleAdapter {
    pub max_offset: u32,
    pub open_tail_floor: f64,
}

impl Default for ManticCrucibleAdapter {
    fn default() -> Self {
        Self {
            max_offset: 1_000,
            open_tail_floor: MANTIC_OPEN_TAIL_FLOOR,
        }
    }
}

impl ManticCrucibleAdapter {
    /// Normalize an [`ApiFilter`] for Mantic's wire quirks:
    /// - empty `allowed_types` is kept empty (never injecting unsupported types),
    /// - `offset` is clamped to `self.max_offset`.
    #[must_use]
    pub fn normalize_filter(&self, filter: &ApiFilter) -> ApiFilter {
        let mut out = filter.clone();
        if let Some(off) = out.offset {
            out.offset = Some(off.min(self.max_offset));
        }
        out
    }

    /// Rewrite a Mantic wire JSON payload onto `MetaculusQuestion`, translating
    /// `"type": "quantitative"` onto `NumericQuestion` or `DiscreteQuestion`.
    pub fn rewrite_wire_question(
        &self,
        payload: &Value,
    ) -> Result<MetaculusQuestion, MetaculusError> {
        let obj = payload
            .as_object()
            .ok_or_else(|| MetaculusError::InvalidManticPayload("expected JSON object".into()))?;
        let raw_type = obj
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| MetaculusError::InvalidManticPayload("missing 'type' field".into()))?;

        let meta = QuestionMeta {
            id: obj.get("id").and_then(Value::as_u64).unwrap_or(0),
            title: obj
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            resolution_criteria: obj
                .get("resolution_criteria")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            open_time_utc: obj
                .get("open_time_utc")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            scheduled_close_time_utc: obj
                .get("scheduled_close_time_utc")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            scheduled_resolve_time_utc: obj
                .get("scheduled_resolve_time_utc")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            spot_scoring_time_utc: obj
                .get("spot_scoring_time_utc")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        };

        match raw_type {
            "binary" => Ok(MetaculusQuestion::Binary(BinaryQuestion { meta })),
            "multiple_choice" => {
                let options = obj
                    .get("options")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        MetaculusError::InvalidManticPayload("missing 'options' array".into())
                    })?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                if options.len() < 2 {
                    return Err(MetaculusError::InvalidManticPayload(
                        "multiple_choice requires >= 2 options".into(),
                    ));
                }
                Ok(MetaculusQuestion::MultipleChoice(MultipleChoiceQuestion {
                    meta,
                    options,
                }))
            }
            "quantitative" | "numeric" | "discrete" => {
                let lower_bound = obj
                    .get("lower_bound")
                    .or_else(|| obj.get("range_min"))
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        MetaculusError::InvalidManticPayload("missing 'lower_bound'".into())
                    })?;
                let upper_bound = obj
                    .get("upper_bound")
                    .or_else(|| obj.get("range_max"))
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        MetaculusError::InvalidManticPayload("missing 'upper_bound'".into())
                    })?;
                let open_lower_bound = obj
                    .get("open_lower_bound")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let open_upper_bound = obj
                    .get("open_upper_bound")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

                if let Some(count) = obj
                    .get("inbound_outcome_count")
                    .or_else(|| obj.get("bin_count"))
                    .and_then(Value::as_u64)
                {
                    Ok(MetaculusQuestion::Discrete(DiscreteQuestion {
                        meta,
                        lower_bound,
                        upper_bound,
                        open_lower_bound,
                        open_upper_bound,
                        inbound_outcome_count: count as usize,
                    }))
                } else {
                    let zero_point = obj.get("zero_point").and_then(Value::as_f64);
                    let unit = obj
                        .get("unit")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    Ok(MetaculusQuestion::Numeric(NumericQuestion {
                        meta,
                        lower_bound,
                        upper_bound,
                        open_lower_bound,
                        open_upper_bound,
                        zero_point,
                        unit,
                    }))
                }
            }
            other => Err(MetaculusError::InvalidManticPayload(format!(
                "unsupported wire type '{other}'"
            ))),
        }
    }

    /// Aggregate per-bin PMFs (`2 <= N <= 31`) across ensemble legs via pointwise mean,
    /// then enforce the `>= open_tail_floor` mass floor on any open tails (`§5.2`, `§11.4`).
    pub fn aggregate_and_floor_pmfs(
        &self,
        legs: &[Vec<f64>],
        open_lower: bool,
        open_upper: bool,
    ) -> Result<Vec<f64>, MetaculusError> {
        let mean = aggregate_mantic_pmfs_pointwise(legs)?;
        enforce_mantic_open_tail_floor(&mean, open_lower, open_upper, self.open_tail_floor)
    }
}

/// Pointwise mean aggregation of per-bin Mantic PMFs (`2 <= N <= 31`).
pub fn aggregate_mantic_pmfs_pointwise(legs: &[Vec<f64>]) -> Result<Vec<f64>, MetaculusError> {
    if legs.is_empty() {
        return Err(MetaculusError::InvalidManticPmf("empty ensemble legs"));
    }
    let n = legs[0].len();
    if !(2..=MANTIC_MAX_BINS).contains(&n) {
        return Err(MetaculusError::InvalidManticPmf(
            "bin count must be in 2..=31",
        ));
    }
    if legs.iter().any(|l| l.len() != n) {
        return Err(MetaculusError::InvalidManticPmf(
            "mismatched bin counts across legs",
        ));
    }

    let mut mean = vec![0.0; n];
    let inv_m = 1.0 / (legs.len() as f64);
    for leg in legs {
        if leg.iter().any(|&v| !v.is_finite() || v < 0.0) {
            return Err(MetaculusError::InvalidManticPmf(
                "pmf entries must be finite and non-negative",
            ));
        }
        let leg_sum: f64 = leg.iter().sum();
        if leg_sum <= 0.0 {
            return Err(MetaculusError::InvalidManticPmf("pmf leg sum must be > 0"));
        }
        for (dst, &v) in mean.iter_mut().zip(leg.iter()) {
            *dst += inv_m * (v / leg_sum);
        }
    }
    Ok(mean)
}

/// Enforce the `>= tail_floor` (default `0.05`) open-tail mass floor on Mantic quantitative PMFs.
pub fn enforce_mantic_open_tail_floor(
    pmf: &[f64],
    open_lower: bool,
    open_upper: bool,
    tail_floor: f64,
) -> Result<Vec<f64>, MetaculusError> {
    let n = pmf.len();
    if !(2..=MANTIC_MAX_BINS).contains(&n) {
        return Err(MetaculusError::InvalidManticPmf(
            "bin count must be in 2..=31",
        ));
    }
    if !tail_floor.is_finite() || tail_floor < 0.0 {
        return Err(MetaculusError::InvalidManticPmf("invalid tail floor"));
    }
    let open_count = usize::from(open_lower) + usize::from(open_upper);
    if (open_count as f64) * tail_floor >= 1.0 {
        return Err(MetaculusError::InvalidManticPmf(
            "open tail floor sum must be < 1.0",
        ));
    }
    if pmf.iter().any(|&v| !v.is_finite() || v < 0.0) {
        return Err(MetaculusError::InvalidManticPmf(
            "pmf entries must be finite and non-negative",
        ));
    }

    let total: f64 = pmf.iter().sum();
    if total <= 0.0 {
        return Err(MetaculusError::InvalidManticPmf("pmf sum must be > 0"));
    }
    let mut p: Vec<f64> = pmf.iter().map(|&v| v / total).collect();

    // Iteratively lock any open tail below tail_floor and renormalize remaining bins.
    let mut lock_lo = false;
    let mut lock_hi = false;
    for _ in 0..3 {
        let mut changed = false;
        if open_lower && !lock_lo && p[0] < tail_floor - 1e-15 {
            p[0] = tail_floor;
            lock_lo = true;
            changed = true;
        }
        if open_upper && !lock_hi && p[n - 1] < tail_floor - 1e-15 {
            p[n - 1] = tail_floor;
            lock_hi = true;
            changed = true;
        }
        if !changed {
            break;
        }
        let locked_sum =
            (if lock_lo { tail_floor } else { 0.0 }) + (if lock_hi { tail_floor } else { 0.0 });
        let rem = 1.0 - locked_sum;
        let free_indices: Vec<usize> = (0..n)
            .filter(|&i| !((i == 0 && lock_lo) || (i == n - 1 && lock_hi)))
            .collect();
        let free_sum: f64 = free_indices.iter().map(|&i| p[i]).sum();
        if free_sum > 0.0 {
            for &i in &free_indices {
                p[i] *= rem / free_sum;
            }
        } else if !free_indices.is_empty() {
            let even = rem / (free_indices.len() as f64);
            for &i in &free_indices {
                p[i] = even;
            }
        }
    }
    Ok(p)
}

/// Published ForecastBench crowd/superforecaster/public median row (`GATE-11`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForecastBenchMedianRow {
    pub question_id: String,
    pub available_utc: String,
    pub freeze_crowd_median: Option<f64>,
    pub sf_median: Option<f64>,
    pub public_median: Option<f64>,
}

/// Provenance tier selected by [`ForecastBenchMedianBackfill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrowdPriorSource {
    VenueLive,
    ForecastBenchMedian,
    ClusterConsensus,
}

/// Resolved crowd prior with explicit provenance tier.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResolvedCrowdPrior {
    pub source: CrowdPriorSource,
    pub prob: ProbClamped,
}

/// 3-tier crowd prior backfill cascade with `INV-03` (`available_utc <= as_of_utc`)
/// and `INV-04` (`blind_pre_seal`) enforcement (`GATE-11`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ForecastBenchMedianBackfill;

impl ForecastBenchMedianBackfill {
    /// Resolve crowd prior across 3 tiers:
    /// 1. Live venue TOB probability (when liquid & unmasked),
    /// 2. Vintage-locked ForecastBench median (`freeze_crowd_median -> sf_median -> public_median`),
    /// 3. Cross-venue cluster consensus median.
    ///
    /// Returns `None` unconditionally when `blind_pre_seal == true` (`INV-04`).
    #[must_use]
    pub fn resolve(
        venue_tob_prob: Option<f64>,
        venue_thin_or_masked: bool,
        fb_median_row: Option<&ForecastBenchMedianRow>,
        cluster_median: Option<f64>,
        as_of_utc: &str,
        blind_pre_seal: bool,
    ) -> Option<ResolvedCrowdPrior> {
        if blind_pre_seal {
            return None;
        }
        if !venue_thin_or_masked
            && let Some(p) = venue_tob_prob
            && let Ok(clamped) = ProbClamped::new(p)
        {
            return Some(ResolvedCrowdPrior {
                source: CrowdPriorSource::VenueLive,
                prob: clamped,
            });
        }
        if let Some(row) = fb_median_row
            && fcast_core::time::is_utc_le(&row.available_utc, as_of_utc)
            && let Some(p) = row
                .freeze_crowd_median
                .or(row.sf_median)
                .or(row.public_median)
            && let Ok(clamped) = ProbClamped::new(p)
        {
            return Some(ResolvedCrowdPrior {
                source: CrowdPriorSource::ForecastBenchMedian,
                prob: clamped,
            });
        }
        if let Some(p) = cluster_median
            && let Ok(clamped) = ProbClamped::new(p)
        {
            return Some(ResolvedCrowdPrior {
                source: CrowdPriorSource::ClusterConsensus,
                prob: clamped,
            });
        }
        None
    }
}
