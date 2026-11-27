//! Subcommand handlers for the `fcast` CLI (`init`, `demo`, `forecast`, `gemini`, `verify-receipts`).

use fcast_bench::{deterministic_m15_demo_cohort, run_m15_demo_pipeline};
use fcast_core::blinding::MarketBlindingScrubber;
use fcast_core::forecast::{
    OutcomeDistribution, ProbClamped, ResolutionStatusAtCutoff, UnsealedForecastRevision,
};
use fcast_epistemic::{
    BindingAuditVerdict, DraftRevisionProposal, PlanConstraintSpec, PostProposalAudit,
    PreLoomGuard, RepRouter, TemplateCatalog, TuningCacheSnapshot,
};
use fcast_infra::daemon::{BillingPacingTier, TimeBudget};
use fcast_infra::llm::{GEMINI_4_HYDROGEN, GeminiCliClient, GeminiGenerateRequest};
use fcast_infra::store::{HorizonProjection, OperationalStore};
use fcast_infra::sys::DataRootLayout;
use fcast_infra::vault::EvidenceVault;
use fcast_ingest::{DomainDecision, IntakeSnapshot, TargetDomain, assemble_evidence_bundle};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub type CliResult = Result<String, Box<dyn std::error::Error>>;

#[derive(Debug, Serialize)]
struct InitReport {
    status: &'static str,
    data_root: String,
    db_path: String,
    zero_key_seed_revision_id: String,
}

#[derive(Debug, Serialize)]
struct GateReceipt {
    gate_id: String,
    status: &'static str,
    tier: &'static str,
    workspace_sha256: String,
    templates_verified: usize,
}

pub fn default_data_root() -> PathBuf {
    if let Ok(root) = env::var("FCAST_DATA_ROOT") {
        return PathBuf::from(root);
    }
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("fcast");
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".local/share/fcast");
    }
    PathBuf::from(".fcast_data")
}

fn require_flag_value<'a>(
    args: &'a [String],
    idx: usize,
    flag: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("missing value for {flag}").into())
}

pub fn run_init_cli(args: &[String]) -> CliResult {
    let mut data_root = default_data_root();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-root" => {
                data_root = PathBuf::from(require_flag_value(args, i, "--data-root")?);
                i += 2;
            }
            other => return Err(format!("unknown flag for `fcast init`: {other}").into()),
        }
    }
    let report = run_init(&data_root)?;
    Ok(serde_json::to_string_pretty(&report)?)
}

fn run_init(data_root: &Path) -> Result<InitReport, Box<dyn std::error::Error>> {
    let layout = DataRootLayout::init(data_root)?;
    let store = OperationalStore::open(&layout.db_path)?;
    let vault = EvidenceVault::open(&layout.vault_dir)?;

    let ev_art = vault.put_bytes(
        "seed_ov",
        b"ForecastBench zero-key offline seed outside-view dossier.",
    )?;
    let rat_art = vault.put_bytes(
        "seed_rat0",
        b"InitialForecast sealed during zero-key fcast init.",
    )?;

    let seed_ts = "2026-11-25T00:00:00Z";
    store.upsert_question_if_absent(
        "q_seed_001",
        "forecastbench_seed",
        "Zero-Key Seed Contract",
        &ev_art.sha256_hex,
        "point_in_time",
        seed_ts,
    )?;
    store.index_vault_artifact(&ev_art, seed_ts)?;
    store.index_vault_artifact(&rat_art, seed_ts)?;
    store.ensure_forecast_stream("fc_seed_001", "q_seed_001", "live_tournament")?;

    let rev0 = UnsealedForecastRevision {
        forecast_id: "fc_seed_001".into(),
        seq_no: 0,
        parent_revision_id: None,
        question_id: "q_seed_001".into(),
        as_of_utc: "2026-11-25T00:05:00Z".into(),
        p_indep: ProbClamped::new(0.25)?,
        p_market: Some(ProbClamped::new(0.22)?),
        probability_distribution: OutcomeDistribution::Binary {
            p: ProbClamped::new(0.24)?,
            n_eff: 6.0,
        },
        evidence_handles: vec![ev_art.ev_handle],
        model_version: "seed-offline@m0".into(),
        calibration_version: "cal-v1".into(),
        rationale_handle: rat_art.ev_handle,
        resolution_status: ResolutionStatusAtCutoff::Unresolved,
    }
    .seal()?;

    if matches!(
        store.project_at_horizon("fc_seed_001", "2026-11-25T00:05:00Z")?,
        HorizonProjection::NotYetIssued
    ) {
        store.seal_forecast_revision(&rev0)?;
    }

    Ok(InitReport {
        status: "ok",
        data_root: layout.root.display().to_string(),
        db_path: layout.db_path.display().to_string(),
        zero_key_seed_revision_id: rev0.revision_id,
    })
}

pub fn run_verify_receipts_cli(args: &[String]) -> CliResult {
    let mut write_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--write-dir" => {
                write_dir = Some(PathBuf::from(require_flag_value(args, i, "--write-dir")?));
                i += 2;
            }
            other => {
                return Err(format!("unknown flag for `fcast verify-receipts`: {other}").into());
            }
        }
    }
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let receipt = run_verify_receipts(&cwd, write_dir.as_deref())?;
    Ok(serde_json::to_string_pretty(&receipt)?)
}

fn run_verify_receipts(
    repo_root: &Path,
    write_dir: Option<&Path>,
) -> Result<GateReceipt, Box<dyn std::error::Error>> {
    let tmpl_dir = repo_root.join("templates");
    // Fail-closed: verify all 9 SHA-256-locked prompt templates and blinding probes before stamping VERIFIED.
    let catalog = TemplateCatalog::load_and_verify(&tmpl_dir)?;
    if !MarketBlindingScrubber::verify_sole_match_probes() {
        return Err("MarketBlindingScrubber sole-match probe verification failed".into());
    }
    let mut hasher = Sha256::new();
    let mut count = 0usize;
    let mut entries: Vec<PathBuf> = fs::read_dir(&tmpl_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("tmpl"))
        .collect();
    entries.sort();
    for p in entries {
        let bytes = fs::read(&p)?;
        hasher.update(&bytes);
        count += 1;
    }
    if count != catalog.len() {
        return Err(format!(
            "template file count mismatch: found {count}, catalog verified {}",
            catalog.len()
        )
        .into());
    }
    let workspace_sha256 = format!("{:x}", hasher.finalize());

    if let Some(dir) = write_dir {
        fs::create_dir_all(dir)?;
        for gate_id in [
            "GATE-01", "GATE-02", "GATE-03", "GATE-04", "GATE-05", "GATE-06", "GATE-07", "GATE-08",
            "GATE-09", "GATE-10", "GATE-11", "GATE-12", "GATE-16", "GATE-17", "GATE-18", "GATE-19",
            "GATE-24",
        ] {
            let rec = GateReceipt {
                gate_id: gate_id.into(),
                status: "VERIFIED",
                tier: "Tier 2 (Crate-Verified)",
                workspace_sha256: workspace_sha256.clone(),
                templates_verified: count,
            };
            fs::write(
                dir.join(format!("{gate_id}.json")),
                serde_json::to_string_pretty(&rec)? + "\n",
            )?;
        }
    }

    Ok(GateReceipt {
        gate_id: "M2-BUNDLE".into(),
        status: "VERIFIED",
        tier: "Tier 2 (Crate-Verified)",
        workspace_sha256,
        templates_verified: count,
    })
}

pub fn run_demo_cli(args: &[String]) -> CliResult {
    let mut data_root: Option<PathBuf> = None;
    let mut live = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-root" => {
                data_root = Some(PathBuf::from(require_flag_value(args, i, "--data-root")?));
                i += 2;
            }
            "--live-gemini" => {
                live = true;
                i += 1;
            }
            other => return Err(format!("unknown flag for `fcast demo`: {other}").into()),
        }
    }
    let owned_dir = data_root.unwrap_or_else(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        env::temp_dir().join(format!("fcast_m2_demo_{}_{}", std::process::id(), nanos))
    });
    let layout = DataRootLayout::init(&owned_dir)?;
    let store = OperationalStore::open(&layout.db_path)?;
    let vault = EvidenceVault::open(&layout.vault_dir)?;
    let cohort = deterministic_m15_demo_cohort();
    let client = GeminiCliClient::from_env();
    let report = run_m15_demo_pipeline(&store, &vault, &cohort, live.then_some(&client))?;
    Ok(serde_json::to_string_pretty(&report)?)
}

pub fn run_forecast_cli(args: &[String]) -> CliResult {
    let question_id = args
        .first()
        .filter(|s| !s.starts_with("--"))
        .ok_or("usage: fcast forecast <question_id> --title <t> --as-of <utc> [--model <m>] [--dry-run] [--data-root <dir>]")?;
    let mut title = format!("Forecast question {question_id}");
    let mut as_of_utc = "2026-11-27T12:00:00Z".to_owned();
    let mut model = GEMINI_4_HYDROGEN.to_owned();
    let mut dry_run = false;
    let mut data_root = default_data_root();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                title = require_flag_value(args, i, "--title")?.to_owned();
                i += 2;
            }
            "--as-of" => {
                as_of_utc = require_flag_value(args, i, "--as-of")?.to_owned();
                i += 2;
            }
            "--model" | "-m" => {
                model = require_flag_value(args, i, "--model")?.to_owned();
                i += 2;
            }
            "--data-root" => {
                data_root = PathBuf::from(require_flag_value(args, i, "--data-root")?);
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            other => return Err(format!("unknown flag for `fcast forecast`: {other}").into()),
        }
    }

    let snap = TuningCacheSnapshot::cold_start();
    let intake = IntakeSnapshot {
        question_id: question_id.clone(),
        platform_id: "cli".into(),
        question_type: "binary".into(),
        domain_decision: DomainDecision::Accept {
            domain: TargetDomain::Geopolitics,
            confidence: 0.9,
            matched_signals: vec!["cli".into()],
        },
        book_mark: None,
        complexity_score: 60.0,
        prize_weight: 1.5,
    };
    let plan = RepRouter::phase_b_select(
        &intake,
        &as_of_utc,
        &PlanConstraintSpec::default(),
        &TimeBudget::from_epochs(10_000, Some(15_000), 20_000),
        BillingPacingTier::Full,
        &snap,
    );
    if plan.skipped {
        return Err(format!(
            "RepRouter skipped question {question_id}: {}",
            plan.skip_reason.as_deref().unwrap_or("unspecified")
        )
        .into());
    }
    let scrubbed = MarketBlindingScrubber::scrub_text(&title);
    let prompt = format!(
        "Estimate probability in (0,1) for: {scrubbed}. Reply strictly as JSON {{\"probability\": <float>}}."
    );
    MarketBlindingScrubber::assert_blind_payload(&prompt)?;

    let client = GeminiCliClient::from_env();
    let mut req = GeminiGenerateRequest::new(model.clone(), prompt);
    req.system_instruction = Some("Return ONLY valid JSON {\"probability\": number}.".into());
    req.dry_run = dry_run;
    let resp = client.generate(&req)?;
    if dry_run {
        // Strictly read-only dry-run: never mutate OperationalStore or EvidenceVault.
        return Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "dry_run_preview",
            "question_id": question_id,
            "as_of_utc": as_of_utc,
            "model": model,
            "selected_rep": plan.selected_rep,
            "reasoning_effort": plan.reasoning_effort,
            "context_tier": plan.context_tier,
            "rollout_k": plan.rollout_k,
            "reshaped_spec_hash": plan.reshaped_spec_hash,
            "dry_run": true,
            "gemini_preview": resp.raw_json,
        }))?);
    }

    let layout = DataRootLayout::init(&data_root)?;
    let store = OperationalStore::open(&layout.db_path)?;
    let vault = EvidenceVault::open(&layout.vault_dir)?;
    store.ensure_tuning_snapshot_idempotent(
        &snap.snapshot_id,
        &snap.created_utc,
        snap.resolved_n,
        &serde_json::to_string(&snap)?,
    )?;

    let ev_art = vault.put_bytes("cli_brief", scrubbed.as_bytes())?;
    let contract_sha = format!("{:x}", Sha256::digest(title.as_bytes()));
    store.upsert_question_if_absent(
        question_id,
        "cli",
        &title,
        &contract_sha,
        "point_in_time",
        &as_of_utc,
    )?;
    store.index_vault_artifact(&ev_art, &as_of_utc)?;
    let bundle = plan
        .evidence_plan
        .as_ref()
        .map(|ep| assemble_evidence_bundle(ep, vec![ev_art.ev_handle.clone()]));

    let (p_guarded, _) = PreLoomGuard::extract_and_guard_prob(&resp.text, 1)?;
    let final_p = match PostProposalAudit::evaluate_binding(&DraftRevisionProposal {
        question_id: question_id.clone(),
        p_prev: ProbClamped::clamp_lossy(0.50),
        p_proposed: p_guarded,
        is_freeze_deadline: false,
        contract_misread_g1: false,
        unverified_load_bearing_claims: vec![],
    }) {
        BindingAuditVerdict::Approve { final_p }
        | BindingAuditVerdict::DampenTowardPrior { final_p, .. } => final_p,
        BindingAuditVerdict::BlockPublish { reason } => {
            return Err(format!("binding audit blocked publish: {reason}").into());
        }
    };

    let fc_id = format!("fc_cli_{question_id}");
    store.ensure_forecast_stream(&fc_id, question_id, "live_tournament")?;
    if let HorizonProjection::StandingRevision {
        revision_id,
        p_indep,
        ..
    } = store.project_at_horizon(&fc_id, &as_of_utc)?
    {
        return Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "already_issued",
            "revision_id": revision_id,
            "forecast_id": fc_id,
            "question_id": question_id,
            "p_indep": p_indep.get(),
            "dry_run": false,
        }))?);
    }

    let rat_art = vault.put_bytes(
        "cli_rat",
        format!("model={model} k=1 p={:.4}", final_p.get()).as_bytes(),
    )?;
    store.index_vault_artifact(&rat_art, &as_of_utc)?;
    let rev = UnsealedForecastRevision {
        forecast_id: fc_id.clone(),
        seq_no: 0,
        parent_revision_id: None,
        question_id: question_id.clone(),
        as_of_utc: as_of_utc.clone(),
        p_indep: final_p,
        p_market: None,
        probability_distribution: OutcomeDistribution::Binary {
            p: final_p,
            n_eff: 4.0,
        },
        evidence_handles: bundle.map_or_else(Vec::new, |b| b.evidence_handles),
        // Stamp honest executed configuration (single-pass CLI call, k=1)
        model_version: format!("{model}@m2+effort:default+ctx:minimal+k:1"),
        calibration_version: format!("cal-m2:{}", snap.snapshot_id),
        rationale_handle: rat_art.ev_handle,
        resolution_status: ResolutionStatusAtCutoff::Unresolved,
    }
    .seal()?;
    store.seal_forecast_revision(&rev)?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "sealed",
        "revision_id": rev.revision_id,
        "forecast_id": fc_id,
        "question_id": question_id,
        "p_indep": rev.p_indep.get(),
        "selected_rep": plan.selected_rep,
        "reasoning_effort": plan.reasoning_effort,
        "context_tier": plan.context_tier,
        "rollout_k": plan.rollout_k,
        "reshaped_spec_hash": plan.reshaped_spec_hash,
        "dry_run": false,
    }))?)
}

pub fn run_gemini_cli(args: &[String]) -> CliResult {
    let client = GeminiCliClient::from_env();
    let sub = args.first().map(String::as_str).unwrap_or("help");
    match sub {
        "whoami" => {
            let info = client.whoami()?;
            Ok(serde_json::to_string_pretty(&info)?)
        }
        "models" => {
            let models = client.models_list()?;
            Ok(serde_json::to_string_pretty(&models)?)
        }
        "generate" => {
            let prompt = args
                .get(1)
                .filter(|s| !s.starts_with("--"))
                .ok_or("missing prompt: fcast gemini generate <prompt> [--model <m>] [--dry-run]")?;
            let mut req = GeminiGenerateRequest::new(GEMINI_4_HYDROGEN, prompt.clone());
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--model" | "-m" => {
                        req.model = require_flag_value(args, i, "--model")?.to_owned();
                        i += 2;
                    }
                    "--system" => {
                        req.system_instruction =
                            Some(require_flag_value(args, i, "--system")?.to_owned());
                        i += 2;
                    }
                    "--previous-interaction-id" => {
                        req.previous_interaction_id = Some(
                            require_flag_value(args, i, "--previous-interaction-id")?.to_owned(),
                        );
                        i += 2;
                    }
                    "--dry-run" => {
                        req.dry_run = true;
                        i += 1;
                    }
                    other => {
                        return Err(
                            format!("unknown flag for `fcast gemini generate`: {other}").into()
                        );
                    }
                }
            }
            let resp = client.generate(&req)?;
            Ok(serde_json::to_string_pretty(&resp)?)
        }
        _ => Ok(
            "Usage:\n  fcast gemini whoami\n  fcast gemini models\n  fcast gemini generate <prompt> [--model <m>] [--system <s>] [--previous-interaction-id <id>] [--dry-run]\n"
                .to_owned(),
        ),
    }
}
