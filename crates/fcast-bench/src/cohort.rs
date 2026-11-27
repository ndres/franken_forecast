//! Time-fenced resolved benchmark cohorts for `M1.5` skill verification & `fcast demo`.

use fcast_ingest::TargetDomain;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Final adjudication status for a benchmark question (`§7.2` Stage 4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BenchmarkOutcome {
    ResolvedBinary { y: bool },
    ResolvedNumeric { actual: f64 },
    Invalidated,
}

/// One time-fenced benchmark question with published baselines and pre-cutoff history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBenchmarkQuestion {
    pub question_id: String,
    pub venue: String,
    pub title: String,
    pub domain: TargetDomain,
    pub freeze_utc: String,
    pub resolved_published_utc: String,
    pub crowd_median: f64,
    pub sf_median: f64,
    pub outcome: BenchmarkOutcome,
    /// Pre-cutoff official history prints (for Thesis persistence baseline & `1.28*sigma` dispersion).
    pub prior_history: Vec<f64>,
    pub seasonal_note: String,
    /// If true, simulates an in-flight abandonment (sunk cost + vaulted failed trace, zero sealed revision).
    pub simulate_abandoned: bool,
    /// Deterministic offline probability for Arm A (`R0` bare single-pass) when `--live-gemini` is off.
    pub arm_r0_offline_p: f64,
    /// Deterministic offline probability for Arm B (`R3` structured + `SEASONAL_NOTE` + binding Red-Team).
    pub arm_r3_offline_p: f64,
}

/// Loads a JSON array of [`ResolvedBenchmarkQuestion`] from disk.
pub fn load_cohort_json(
    path: impl AsRef<Path>,
) -> Result<Vec<ResolvedBenchmarkQuestion>, std::io::Error> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Deterministic `12`-question time-fenced cohort for `M1.5` (`10` resolved binary questions
/// across Geopolitics/Macro/Cyber, `1` `Invalidated` void question for `GATE-06`, and `1`
/// intentionally `Abandoned` question with vaulted failure trace for `INV-13`).
#[must_use]
pub fn deterministic_m15_demo_cohort() -> Vec<ResolvedBenchmarkQuestion> {
    vec![
        ResolvedBenchmarkQuestion {
            question_id: "m15_geo_hormuz".into(),
            venue: "forecastbench".into(),
            title: "Will Strait of Hormuz commercial transit close for >48h before 2026-11-15?".into(),
            domain: TargetDomain::Geopolitics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-16T00:00:00Z".into(),
            crowd_median: 0.24,
            sf_median: 0.16,
            outcome: BenchmarkOutcome::ResolvedBinary { y: false },
            prior_history: vec![0.10, 0.12, 0.14, 0.11, 0.13, 0.12],
            seasonal_note: "5-yr baseline: 0 closures >48h despite 4 regional escalations.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.28,
            arm_r3_offline_p: 0.12,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_macro_boj".into(),
            venue: "metaculus".into(),
            title: "Will the Bank of Japan raise its policy rate by >=25 bps in Q4 2026?".into(),
            domain: TargetDomain::Macroeconomics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-20T00:00:00Z".into(),
            crowd_median: 0.62,
            sf_median: 0.70,
            outcome: BenchmarkOutcome::ResolvedBinary { y: true },
            prior_history: vec![0.10, 0.25, 0.25, 0.50, 0.50, 0.75],
            seasonal_note: "3-yr core CPI above 2.0% target; Q4 wage-pass-through window.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.58,
            arm_r3_offline_p: 0.74,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_cyber_eu_grid".into(),
            venue: "forecastbench".into(),
            title: "Will a state-attributed cyber incident cause >=100 MW EU grid outage in Oct-Nov 2026?".into(),
            domain: TargetDomain::Cybersecurity,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-22T00:00:00Z".into(),
            crowd_median: 0.22,
            sf_median: 0.14,
            outcome: BenchmarkOutcome::ResolvedBinary { y: false },
            prior_history: vec![0.02, 0.03, 0.02, 0.04, 0.03, 0.03],
            seasonal_note: "EuRepoC 5-yr winter pre-heating window: 0 >100 MW grid outages in EU.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.25,
            arm_r3_offline_p: 0.10,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_macro_fed_cut".into(),
            venue: "metaculus".into(),
            title: "Will the Federal Reserve cut the federal funds target range at the Nov 2026 FOMC?".into(),
            domain: TargetDomain::Macroeconomics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-08T00:00:00Z".into(),
            crowd_median: 0.68,
            sf_median: 0.75,
            outcome: BenchmarkOutcome::ResolvedBinary { y: true },
            prior_history: vec![5.25, 5.00, 4.75, 4.50, 4.25, 4.00],
            seasonal_note: "CME FedWatch + ALFRED PCE 3-yr disinflation trajectory.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.64,
            arm_r3_offline_p: 0.78,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_geo_ceasefire".into(),
            venue: "forecastbench".into(),
            title: "Will a formal bilateral 30-day ceasefire take effect in Eastern Europe before 2026-11-20?".into(),
            domain: TargetDomain::Geopolitics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-21T00:00:00Z".into(),
            crowd_median: 0.30,
            sf_median: 0.21,
            outcome: BenchmarkOutcome::ResolvedBinary { y: false },
            prior_history: vec![0.18, 0.22, 0.15, 0.20, 0.17, 0.16],
            seasonal_note: "5-yr autumn mud-season operational tempo: 0 formal 30-day truces.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.33,
            arm_r3_offline_p: 0.18,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_cyber_kev_surge".into(),
            venue: "metaculus".into(),
            title: "Will CISA add >=12 new vulnerabilities to the KEV catalog during November 2026?".into(),
            domain: TargetDomain::Cybersecurity,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-25T00:00:00Z".into(),
            crowd_median: 0.58,
            sf_median: 0.66,
            outcome: BenchmarkOutcome::ResolvedBinary { y: true },
            prior_history: vec![14.0, 11.0, 16.0, 13.0, 15.0, 18.0],
            seasonal_note: "CISA KEV 3-yr November median = 14.5 additions (Patch Tuesday spike).".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.54,
            arm_r3_offline_p: 0.72,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_macro_cpi_above".into(),
            venue: "forecastbench".into(),
            title: "Will US Headline CPI YoY (Oct 2026 first print) exceed 2.8%?".into(),
            domain: TargetDomain::Macroeconomics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-13T13:30:00Z".into(),
            crowd_median: 0.36,
            sf_median: 0.28,
            outcome: BenchmarkOutcome::ResolvedBinary { y: false },
            prior_history: vec![2.9, 2.7, 2.6, 2.5, 2.4, 2.4],
            seasonal_note: "BLS CPI Oct seasonal gasoline drag averaged -0.18pp over past 5 yrs.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.39,
            arm_r3_offline_p: 0.24,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_geo_tariff_pass".into(),
            venue: "metaculus".into(),
            title: "Will the EU finalize retaliatory semiconductor tariffs before 2026-11-25?".into(),
            domain: TargetDomain::Geopolitics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-25T18:00:00Z".into(),
            crowd_median: 0.42,
            sf_median: 0.32,
            outcome: BenchmarkOutcome::ResolvedBinary { y: false },
            prior_history: vec![0.35, 0.38, 0.34, 0.31, 0.29, 0.27],
            seasonal_note: "EU Trade Council legislative timelines median 14 weeks post-draft.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.46,
            arm_r3_offline_p: 0.29,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_cyber_bgp_hijack".into(),
            venue: "forecastbench".into(),
            title: "Will Cloudflare Radar log a Tier-1 ASN BGP route leak impacting >5% global prefixes in Nov 2026?".into(),
            domain: TargetDomain::Cybersecurity,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-25T23:00:00Z".into(),
            crowd_median: 0.19,
            sf_median: 0.13,
            outcome: BenchmarkOutcome::ResolvedBinary { y: false },
            prior_history: vec![0.08, 0.11, 0.09, 0.12, 0.10, 0.09],
            seasonal_note: "RPKI ROV deployment >55% reduced Tier-1 >5% leaks to <1/yr.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.22,
            arm_r3_offline_p: 0.11,
        },
        ResolvedBenchmarkQuestion {
            question_id: "m15_macro_unemp_low".into(),
            venue: "metaculus".into(),
            title: "Will US U-3 Unemployment Rate (Oct 2026 first print) remain <=4.4%?".into(),
            domain: TargetDomain::Macroeconomics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-06T13:30:00Z".into(),
            crowd_median: 0.72,
            sf_median: 0.79,
            outcome: BenchmarkOutcome::ResolvedBinary { y: true },
            prior_history: vec![4.1, 4.2, 4.1, 4.2, 4.1, 4.2],
            seasonal_note: "BLS U-3 1-month std dev = 0.08pp over trailing 6 prints.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.68,
            arm_r3_offline_p: 0.82,
        },
        // GATE-06 Void / Ambiguous resolution: MUST be excluded from Brier/Log denominators
        // while keeping forecast_revisions.resolution_status == 'unresolved' intact!
        ResolvedBenchmarkQuestion {
            question_id: "m15_void_ambiguous".into(),
            venue: "metaculus".into(),
            title: "Will the disputed maritime transit annex be ratified under unclear criteria?".into(),
            domain: TargetDomain::Geopolitics,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-18T00:00:00Z".into(),
            crowd_median: 0.50,
            sf_median: 0.50,
            outcome: BenchmarkOutcome::Invalidated,
            prior_history: vec![0.50, 0.50, 0.50],
            seasonal_note: "Ambiguous clause; later voided by platform moderators.".into(),
            simulate_abandoned: false,
            arm_r0_offline_p: 0.55,
            arm_r3_offline_p: 0.48,
        },
        // INV-13 Abandoned in-flight question: sunk cost + vaulted failure trace, zero sealed revision!
        ResolvedBenchmarkQuestion {
            question_id: "m15_abandoned_baltic".into(),
            venue: "forecastbench".into(),
            title: "Will official attribution name a state actor for Baltic subsea cable damage before 2026-11-20?".into(),
            domain: TargetDomain::Cybersecurity,
            freeze_utc: "2026-10-15T00:00:00Z".into(),
            resolved_published_utc: "2026-11-21T00:00:00Z".into(),
            crowd_median: 0.35,
            sf_median: 0.30,
            outcome: BenchmarkOutcome::ResolvedBinary { y: false },
            prior_history: vec![0.20, 0.25, 0.22, 0.24, 0.21, 0.23],
            seasonal_note: "Right-censored maritime forensics lag; simulated mid-flight abort.".into(),
            simulate_abandoned: true,
            arm_r0_offline_p: 0.50,
            arm_r3_offline_p: 0.50,
        },
    ]
}
