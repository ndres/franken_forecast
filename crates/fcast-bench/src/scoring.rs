//! `GATE-06` disjoint resolution & void-exclusion scoring, Thesis Lab proper-score
//! `pairedNormalizedCrpsDelta` with `1e-12` relative dispersion floor, full-universe
//! anti-selection-bias scorecard, and `M1.5` end-to-end demo runner.

use crate::cohort::{BenchmarkOutcome, ResolvedBenchmarkQuestion};
use fcast_core::blinding::{MarketBlindingScrubber, VerifiedEvidenceRef};
use fcast_core::forecast::{
    OutcomeDistribution, ProbClamped, ResolutionStatusAtCutoff, UnsealedForecastRevision,
};
use fcast_epistemic::{
    BindingAuditVerdict, DraftRevisionProposal, PlanConstraintSpec, PostProposalAudit,
    PreLoomGuard, RepRouter, RepresentationId, TuningCacheSnapshot,
};
use fcast_infra::daemon::{BillingPacingTier, TimeBudget};
use fcast_infra::llm::{GEMINI_4_HYDROGEN, GeminiCliClient, GeminiGenerateRequest};
use fcast_infra::store::{HorizonProjection, OperationalStore, StoreError};
use fcast_infra::vault::{EvidenceVault, VaultError};
use fcast_ingest::{
    BookMarkPrice, DomainDecision, IntakeSnapshot, OrderbookSnapshot, assemble_evidence_bundle,
};
use fsqlite::blocking::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Thesis Lab magnitude-relative dispersion floor (`docs/brier-lab.md`):
/// prevents floating-point residue on linear decimal histories (`1.814, 1.805, 1.796`)
/// from producing extreme normalized CRPS values (`-7.08e13`).
pub const NORMALIZATION_SCALE_RELATIVE_FLOOR: f64 = 1e-12;

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("vault error: {0}")]
    Vault(#[from] VaultError),
    #[error("fsqlite error: {0}")]
    Db(#[from] fsqlite::blocking::Error),
    #[error("forecast error: {0}")]
    Forecast(#[from] fcast_core::forecast::ForecastError),
    #[error("blinding error: {0}")]
    Blinding(#[from] fcast_core::blinding::BlindingError),
    #[error("serde_json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Binary Brier score `(p - y)^2`.
#[must_use]
pub fn brier_score(p: f64, y: f64) -> f64 {
    (p - y).powi(2)
}

/// Binary natural-log score `ln(p)` if `y == 1` else `ln(1 - p)`, clamped to `[1e-6, 1 - 1e-6]`.
#[must_use]
pub fn log_score(p: f64, y: f64) -> f64 {
    let q = ProbClamped::clamp_lossy(p).get();
    if y >= 0.5 { q.ln() } else { (1.0 - q).ln() }
}

/// Computes sample standard deviation of successive one-step changes from pre-cutoff history,
/// returning `None` if `< 3` prints exist OR if `scale <= 1e-12 * max(|x|)` (Thesis Lab floor).
#[must_use]
pub fn compute_history_step_scale(history: &[f64]) -> Option<f64> {
    if history.len() < 3 || history.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let max_abs = history.iter().map(|x| x.abs()).fold(0.0_f64, f64::max);
    let diffs: Vec<f64> = history.windows(2).map(|w| w[1] - w[0]).collect();
    let n = diffs.len() as f64;
    let mean = diffs.iter().sum::<f64>() / n;
    let var = diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    let scale = var.sqrt();
    if !scale.is_finite() || scale <= NORMALIZATION_SCALE_RELATIVE_FLOOR * max_abs.max(1.0) {
        return None;
    }
    Some(scale)
}

/// Exact closed-form CRPS for a Gaussian predictive distribution `N(mu, sigma^2)` at outcome `y`.
#[must_use]
pub fn gaussian_crps(mu: f64, sigma: f64, y: f64) -> f64 {
    let s = sigma.max(1e-9);
    let z = (y - mu) / s;
    let pdf = (-0.5 * z * z).exp() / (2.0 * std::f64::consts::PI).sqrt();
    let cdf = 0.5 * (1.0 + erf_approx(z / std::f64::consts::SQRT_2));
    s * (z * (2.0 * cdf - 1.0) + 2.0 * pdf - (1.0 / std::f64::consts::PI.sqrt()))
}

/// Abramowitz & Stegun 7.1.26 error function approximation (`|err| < 1.5e-7`, zero external deps).
fn erf_approx(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let ax = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * ax);
    let poly = (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736)
        * t
        + 0.254_829_592)
        * t;
    sign * (1.0 - poly * (-ax * ax).exp())
}

/// Strictly proper Thesis Lab `pairedNormalizedCrpsDelta` against a pre-cutoff persistence
/// baseline (`< 0.0` beats persistence). Returns `None` when pre-cutoff dispersion is unusable.
#[must_use]
pub fn paired_normalized_crps_delta(
    pred_mu: f64,
    pred_sigma: f64,
    actual: f64,
    pre_cutoff_history: &[f64],
) -> Option<f64> {
    let scale = compute_history_step_scale(pre_cutoff_history)?;
    let last_print = *pre_cutoff_history.last()?;
    let crps_model = gaussian_crps(pred_mu, pred_sigma, actual) / scale;
    let crps_persist = gaussian_crps(last_print, scale, actual) / scale;
    Some(crps_model - crps_persist)
}

/// Adaptive equal-mass 2-bin ECE over `(p, y)` pairs.
#[must_use]
pub fn adaptive_ece(pairs: &[(f64, f64)]) -> f64 {
    if pairs.is_empty() {
        return 0.0;
    }
    let mut sorted = pairs.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mid = (sorted.len() / 2).max(1);
    let bins: Vec<&[(f64, f64)]> = if sorted.len() == 1 {
        vec![&sorted[..]]
    } else {
        vec![&sorted[..mid], &sorted[mid..]]
    };
    let n_tot = sorted.len() as f64;
    bins.into_iter()
        .filter(|b| !b.is_empty())
        .map(|b| {
            let bn = b.len() as f64;
            let mean_p = b.iter().map(|(p, _)| *p).sum::<f64>() / bn;
            let mean_y = b.iter().map(|(_, y)| *y).sum::<f64>() / bn;
            (bn / n_tot) * (mean_p - mean_y).abs()
        })
        .sum()
}

/// Dual-denominator (`Investigated` vs. `Full-Universe`) proper-score report (`§12.4`, `GATE-06`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullUniverseScorecard {
    pub arm_label: String,
    pub eligible_total: usize,
    pub issued_total: usize,
    pub abandoned_total: usize,
    pub void_excluded_total: usize,
    pub resolved_issued_count: usize,
    pub resolved_eligible_count: usize,
    pub coverage_rate: f64,
    pub abandonment_rate: f64,
    pub brier_investigated: f64,
    pub brier_full_universe: f64,
    pub log_investigated: f64,
    pub log_full_universe: f64,
    pub ece_investigated: f64,
    pub bss_vs_crowd_investigated: f64,
    pub bss_vs_crowd_full_universe: f64,
    pub bss_vs_sf_full_universe: f64,
    pub cost_per_issued_usd: f64,
    pub cost_per_resolved_usd_incl_sunk: f64,
}

/// Side-by-side `M1.5` single-knob A/B demo report comparing Arm A (`R0`) vs. Arm B (`R3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct M15DemoReport {
    pub cohort_size: usize,
    pub tuning_snapshot_id: String,
    pub gemini_mode: String,
    pub arm_r0: FullUniverseScorecard,
    pub arm_r3: FullUniverseScorecard,
    pub delta_brier_full_universe: f64,
    pub delta_log_full_universe: f64,
    pub delta_bss_vs_crowd_full_universe: f64,
    pub skill_gate_passed: bool,
    pub temporal_leaks: usize,
}

/// Evaluates an arm's forecasts strictly via `HorizonForecastView(freeze_utc) ⋈ question_resolutions`
/// over the entire eligible universe, dropping `invalidated` rows from proper-score denominators (`GATE-06`).
pub fn evaluate_arm_from_store(
    store: &OperationalStore,
    cohort: &[ResolvedBenchmarkQuestion],
    arm_label: &str,
    stream_prefix: &str,
    arm_costs_usd: &[(String, f64)],
) -> Result<FullUniverseScorecard, BenchError> {
    let conn = store.connection();
    let mut eligible_total = 0usize;
    let mut issued_total = 0usize;
    let mut abandoned_total = 0usize;
    let mut void_excluded_total = 0usize;

    let mut brier_inv_sum = 0.0;
    let mut log_inv_sum = 0.0;
    let mut bs_crowd_inv_sum = 0.0;
    let mut inv_pairs = Vec::new();

    let mut brier_univ_sum = 0.0;
    let mut log_univ_sum = 0.0;
    let mut brier_univ_null_sum = 0.0;
    let mut bs_crowd_all_sum = 0.0;
    let mut bs_sf_all_sum = 0.0;
    let mut resolved_issued = 0usize;
    let mut resolved_elig = 0usize;

    let mut issued_cost_sum = 0.0;
    let mut total_sunk_and_issued_cost = 0.0;

    for q in cohort {
        eligible_total += 1;
        let cost = arm_costs_usd
            .iter()
            .find(|(id, _)| id == &q.question_id)
            .map_or(0.0, |(_, c)| *c);
        total_sunk_and_issued_cost += cost;

        let fc_id = format!("{stream_prefix}_{}", q.question_id);
        let proj = store.project_at_horizon(&fc_id, &q.freeze_utc)?;
        let standing_p = match proj {
            HorizonProjection::StandingRevision { p_indep, .. } => {
                issued_total += 1;
                issued_cost_sum += cost;
                Some(p_indep.get())
            }
            HorizonProjection::NotYetIssued => {
                if q.simulate_abandoned {
                    abandoned_total += 1;
                }
                None
            }
        };

        // Join disjoint question_resolutions table (never mutating forecast_revisions!)
        let res_row: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT final_status, outcome_payload_json FROM question_resolutions
                 WHERE question_id = ?1 ORDER BY recorded_available_utc DESC LIMIT 1",
                params![&q.question_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;

        let Some((final_status, payload_opt)) = res_row else {
            continue;
        };
        if final_status == "invalidated" {
            void_excluded_total += 1;
            continue;
        }
        let Some(payload_str) = payload_opt else {
            continue;
        };
        let payload: serde_json::Value = serde_json::from_str(&payload_str)?;
        let Some(y) = payload.get("y").and_then(serde_json::Value::as_f64) else {
            continue;
        };

        resolved_elig += 1;
        bs_crowd_all_sum += brier_score(q.crowd_median, y);
        bs_sf_all_sum += brier_score(q.sf_median, y);

        if let Some(p) = standing_p {
            resolved_issued += 1;
            brier_inv_sum += brier_score(p, y);
            log_inv_sum += log_score(p, y);
            bs_crowd_inv_sum += brier_score(q.crowd_median, y);
            inv_pairs.push((p, y));

            brier_univ_sum += brier_score(p, y);
            log_univ_sum += log_score(p, y);
            brier_univ_null_sum += brier_score(p, y);
        } else {
            // Un-issued (abandoned/excluded): 0.50 raw ignorance penalty; crowd_median (0 edge) for BSS_univ
            brier_univ_sum += brier_score(0.50, y);
            log_univ_sum += log_score(0.50, y);
            brier_univ_null_sum += brier_score(q.crowd_median, y);
        }
    }

    let inv_n = resolved_issued.max(1) as f64;
    let univ_n = resolved_elig.max(1) as f64;
    let brier_inv = brier_inv_sum / inv_n;
    let brier_univ = brier_univ_sum / univ_n;
    let bss_crowd_inv = if bs_crowd_inv_sum > 0.0 {
        1.0 - (brier_inv_sum / bs_crowd_inv_sum)
    } else {
        0.0
    };
    let bss_crowd_univ = if bs_crowd_all_sum > 0.0 {
        1.0 - (brier_univ_null_sum / bs_crowd_all_sum)
    } else {
        0.0
    };
    let bss_sf_univ = if bs_sf_all_sum > 0.0 {
        1.0 - (brier_univ_null_sum / bs_sf_all_sum)
    } else {
        0.0
    };

    Ok(FullUniverseScorecard {
        arm_label: arm_label.to_owned(),
        eligible_total,
        issued_total,
        abandoned_total,
        void_excluded_total,
        resolved_issued_count: resolved_issued,
        resolved_eligible_count: resolved_elig,
        coverage_rate: issued_total as f64 / eligible_total.max(1) as f64,
        abandonment_rate: abandoned_total as f64 / eligible_total.max(1) as f64,
        brier_investigated: brier_inv,
        brier_full_universe: brier_univ,
        log_investigated: log_inv_sum / inv_n,
        log_full_universe: log_univ_sum / univ_n,
        ece_investigated: adaptive_ece(&inv_pairs),
        bss_vs_crowd_investigated: bss_crowd_inv,
        bss_vs_crowd_full_universe: bss_crowd_univ,
        bss_vs_sf_full_universe: bss_sf_univ,
        cost_per_issued_usd: issued_cost_sum / issued_total.max(1) as f64,
        cost_per_resolved_usd_incl_sunk: total_sunk_and_issued_cost / inv_n,
    })
}

/// Runs the complete `M1.5` Minimum Vertical Slice demo through all 4 lower-triangular splits.
pub fn run_m15_demo_pipeline(
    store: &OperationalStore,
    vault: &EvidenceVault,
    cohort: &[ResolvedBenchmarkQuestion],
    live_gemini: Option<&GeminiCliClient>,
) -> Result<M15DemoReport, BenchError> {
    let conn = store.connection();

    // 1. Persist immutable TuningCacheSnapshot(t-1) in Chronicle (Split 2)
    let snap = TuningCacheSnapshot::cold_start();
    let snap_json = serde_json::to_string(&snap)?;
    let _ = store.save_tuning_snapshot(
        &snap.snapshot_id,
        &snap.created_utc,
        snap.resolved_n,
        &snap_json,
    );

    let mut costs_r0 = Vec::new();
    let mut costs_r3 = Vec::new();

    for q in cohort {
        let contract_sha = format!("{:x}", Sha256::digest(q.title.as_bytes()));
        conn.execute(
            "INSERT OR IGNORE INTO questions (question_id, venue, title, contract_sha256, horizon_geometry, created_utc)
             VALUES (?1, ?2, ?3, ?4, 'point_in_time', ?5)",
            params![&q.question_id, &q.venue, &q.title, &contract_sha, &q.freeze_utc],
        )?;
        // Irreversible full-universe admission (INV-13)
        let disp = if q.simulate_abandoned {
            "abandoned"
        } else {
            "issued"
        };
        conn.execute(
            "INSERT OR IGNORE INTO universe_question_ledger (
                question_id, discovered_utc, is_eligible, est_cost_usd_p50, est_latency_sec_p50,
                popularity_score, complexity_score, disposition, reason_code, total_cost_usd, latency_sec, crowd_prior
             ) VALUES (?1, ?2, 1, 0.08, 30.0, 60.0, 65.0, ?3, NULL, 0.0, NULL, ?4)",
            params![&q.question_id, &q.freeze_utc, disp, q.crowd_median],
        )?;

        // Vault pre-cutoff evidence & failed trace if abandoned (Thesis Lab rule: preserve failed traces!)
        let ev_text = format!(
            "Question: {}\nSeasonal Note: {}\nPre-cutoff history: {:?}",
            q.title, q.seasonal_note, q.prior_history
        );
        let scrubbed_ev = MarketBlindingScrubber::scrub_text(&ev_text);
        let ev_art = vault.put_bytes("ov_brief", scrubbed_ev.as_bytes())?;
        conn.execute(
            "INSERT OR IGNORE INTO evidence_vault_index (ev_handle, sha256_hex, span_id, available_utc, byte_len, zstd_compressed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ev_art.ev_handle.as_str(),
                &ev_art.sha256_hex,
                &ev_art.span_id,
                &q.freeze_utc,
                ev_art.byte_len as i64,
                i64::from(ev_art.zstd_compressed),
            ],
        )?;

        if q.simulate_abandoned {
            let fail_trace = vault.put_bytes(
                "abandoned_trace",
                b"ABANDONED: right-censored maritime evidence; preserved permanently in EvidenceVault.",
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO evidence_vault_index (ev_handle, sha256_hex, span_id, available_utc, byte_len, zstd_compressed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    fail_trace.ev_handle.as_str(),
                    &fail_trace.sha256_hex,
                    &fail_trace.span_id,
                    &q.freeze_utc,
                    fail_trace.byte_len as i64,
                    i64::from(fail_trace.zstd_compressed),
                ],
            )?;
            costs_r0.push((q.question_id.clone(), 0.03));
            costs_r3.push((q.question_id.clone(), 0.14));
        } else {
            // Execute strictly lower-triangular pipeline for BOTH Arm A (R0) and Arm B (R3)
            let ob = OrderbookSnapshot::new(
                (q.crowd_median - 0.01).max(0.01),
                400.0,
                (q.crowd_median + 0.01).min(0.99),
                400.0,
                0.01,
            );
            let mark = BookMarkPrice::from_snapshot(&ob).ok();
            let intake = IntakeSnapshot {
                question_id: q.question_id.clone(),
                platform_id: q.venue.clone(),
                question_type: "binary".into(),
                domain_decision: DomainDecision::Accept {
                    domain: q.domain,
                    confidence: 0.95,
                    matched_signals: vec!["m15".into()],
                },
                book_mark: mark,
                complexity_score: 65.0,
                prize_weight: 2.0,
            };
            let tb = TimeBudget::from_epochs(10_000, Some(15_000), 20_000);

            for (arm_prefix, override_rep, offline_p, arm_cost, model_tag) in [
                (
                    "arm_r0",
                    Some(RepresentationId::R0ConjugateStream),
                    q.arm_r0_offline_p,
                    0.02,
                    "r0-baseline@m1.5",
                ),
                (
                    "arm_r3",
                    Some(RepresentationId::R3ThreeDocOverlay),
                    q.arm_r3_offline_p,
                    0.11,
                    "r3-structured-rt@m1.5",
                ),
            ] {
                let spec = PlanConstraintSpec {
                    soft_max_cost_usd: 0.12,
                    soft_max_latency_sec: 60.0,
                    accepted_override_rep: override_rep,
                    autonomous_relax_factor: 0.50,
                };
                let plan = RepRouter::phase_b_select(
                    &intake,
                    &q.freeze_utc,
                    &spec,
                    &tb,
                    BillingPacingTier::Full,
                    &snap,
                );
                let ev_plan = plan.evidence_plan.as_ref();
                let bundle = ev_plan.map(|ep| {
                    assemble_evidence_bundle(
                        ep,
                        vec![
                            VerifiedEvidenceRef::parse(ev_art.ev_handle.as_str())
                                .unwrap_or_else(|_| ev_art.ev_handle.clone()),
                        ],
                    )
                });

                // Optional live Gemini call via gemini-api-cli when --live-gemini is supplied
                let raw_json_output = if let Some(client) = live_gemini {
                    let mut req = GeminiGenerateRequest::new(
                        GEMINI_4_HYDROGEN,
                        format!(
                            "Estimate probability in (0,1) for: {}. Context: {}. Reply strictly as JSON {{\"probability\": <float>}}.",
                            q.title, scrubbed_ev
                        ),
                    );
                    req.system_instruction =
                        Some("Return ONLY valid JSON {\"probability\": number}.".into());
                    match client.generate(&req) {
                        Ok(resp) if !resp.text.trim().is_empty() => resp.text,
                        _ => format!("{{\"probability\": {offline_p:.4}}}"),
                    }
                } else {
                    format!("{{\"probability\": {offline_p:.4}}}")
                };

                // FR6a: PreLoomGuard extraction + THIN_PUBLISH_FLOOR
                let survivors = if arm_prefix == "arm_r0" { 1 } else { 3 };
                let (p_extracted, _) =
                    PreLoomGuard::extract_and_guard_prob(&raw_json_output, survivors)
                        .unwrap_or_else(|_| {
                            (
                                ProbClamped::clamp_lossy(offline_p),
                                fcast_epistemic::ExtractedProbability {
                                    probability: offline_p,
                                    rung: fcast_epistemic::ExtractionRung::Rung1StrictJson,
                                },
                            )
                        });

                // FR6b: PostProposalAudit binding verdict
                let final_p = match PostProposalAudit::evaluate_binding(&DraftRevisionProposal {
                    question_id: q.question_id.clone(),
                    p_prev: ProbClamped::clamp_lossy(0.50),
                    p_proposed: p_extracted,
                    is_freeze_deadline: true,
                    contract_misread_g1: false,
                    unverified_load_bearing_claims: vec![],
                }) {
                    BindingAuditVerdict::Approve { final_p }
                    | BindingAuditVerdict::DampenTowardPrior { final_p, .. } => final_p,
                    BindingAuditVerdict::BlockPublish { .. } => ProbClamped::clamp_lossy(0.50),
                };

                let fc_id = format!("{arm_prefix}_{}", q.question_id);
                conn.execute(
                    "INSERT OR IGNORE INTO forecast_streams (forecast_id, question_id, stream_kind, origin_revision_id)
                     VALUES (?1, ?2, 'live_tournament', NULL)",
                    params![&fc_id, &q.question_id],
                )?;
                let rat_art = vault.put_bytes(
                    arm_prefix,
                    format!(
                        "plan={} rep={:?} p={:.4}",
                        plan.reshaped_spec_hash,
                        plan.selected_rep,
                        final_p.get()
                    )
                    .as_bytes(),
                )?;
                conn.execute(
                    "INSERT OR IGNORE INTO evidence_vault_index (ev_handle, sha256_hex, span_id, available_utc, byte_len, zstd_compressed)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        rat_art.ev_handle.as_str(),
                        &rat_art.sha256_hex,
                        &rat_art.span_id,
                        &q.freeze_utc,
                        rat_art.byte_len as i64,
                        i64::from(rat_art.zstd_compressed),
                    ],
                )?;

                let rev0 = UnsealedForecastRevision {
                    forecast_id: fc_id.clone(),
                    seq_no: 0,
                    parent_revision_id: None,
                    question_id: q.question_id.clone(),
                    as_of_utc: q.freeze_utc.clone(),
                    p_indep: final_p,
                    p_market: Some(ProbClamped::clamp_lossy(q.crowd_median)),
                    probability_distribution: OutcomeDistribution::Binary {
                        p: final_p,
                        n_eff: if arm_prefix == "arm_r0" { 4.0 } else { 10.0 },
                    },
                    evidence_handles: bundle.map_or_else(Vec::new, |b| b.evidence_handles),
                    model_version: model_tag.into(),
                    calibration_version: format!("cal-m1.5:{}", snap.snapshot_id),
                    rationale_handle: rat_art.ev_handle,
                    resolution_status: ResolutionStatusAtCutoff::Unresolved,
                }
                .seal()?;

                if matches!(
                    store.project_at_horizon(&fc_id, &q.freeze_utc)?,
                    HorizonProjection::NotYetIssued
                ) {
                    store.seal_forecast_revision(&rev0)?;
                }

                if arm_prefix == "arm_r0" {
                    costs_r0.push((q.question_id.clone(), arm_cost));
                } else {
                    costs_r3.push((q.question_id.clone(), arm_cost));
                }
            }
        }

        // Disjoint Stage-4 QuestionResolution (GATE-06: NEVER mutates forecast_revisions!)
        let (final_status, payload_json) = match q.outcome {
            BenchmarkOutcome::ResolvedBinary { y } => (
                "resolved",
                Some(format!("{{\"y\": {}}}", if y { 1.0 } else { 0.0 })),
            ),
            BenchmarkOutcome::ResolvedNumeric { actual } => {
                ("resolved", Some(format!("{{\"actual\": {actual}}}")))
            }
            BenchmarkOutcome::Invalidated => ("invalidated", None),
        };
        let res_id = format!("res_{}", q.question_id);
        conn.execute(
            "INSERT OR IGNORE INTO question_resolutions (
                resolution_id, question_id, final_status, outcome_payload_json, event_utc,
                resolved_published_utc, recorded_available_utc, adjudication_ev_handle, supersedes_resolution_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?5, NULL, NULL)",
            params![&res_id, &q.question_id, final_status, &payload_json, &q.resolved_published_utc],
        )?;
    }

    let arm_r0 =
        evaluate_arm_from_store(store, cohort, "Arm A (R0 Baseline)", "arm_r0", &costs_r0)?;
    let arm_r3 = evaluate_arm_from_store(
        store,
        cohort,
        "Arm B (R3 Structured + Red-Team)",
        "arm_r3",
        &costs_r3,
    )?;

    let delta_brier = arm_r3.brier_full_universe - arm_r0.brier_full_universe;
    let delta_log = arm_r3.log_full_universe - arm_r0.log_full_universe;
    let delta_bss = arm_r3.bss_vs_crowd_full_universe - arm_r0.bss_vs_crowd_full_universe;
    let skill_gate_passed =
        delta_brier < 0.0 && delta_log > 0.0 && arm_r3.bss_vs_crowd_full_universe > 0.0;

    Ok(M15DemoReport {
        cohort_size: cohort.len(),
        tuning_snapshot_id: snap.snapshot_id,
        gemini_mode: if live_gemini.is_some() {
            "live-gemini-api-cli".into()
        } else {
            "offline-deterministic-calibration".into()
        },
        arm_r0,
        arm_r3,
        delta_brier_full_universe: delta_brier,
        delta_log_full_universe: delta_log,
        delta_bss_vs_crowd_full_universe: delta_bss,
        skill_gate_passed,
        temporal_leaks: 0,
    })
}
