//! `LlmRouter` Gemini API adapter backed by the official `google-gemini/gemini-api-cli`
//! (`gemini-api` binary, `§15`, `Plan v2.1`).
//!
//! Keeps `fcast-infra` 100% pure Rust with zero banned HTTP/async crates while giving `fcast`
//! full access to the Gemini Interactions API (`generate`, `analyze`, `agent run/status`,
//! `models list`, `whoami`, `--previous-interaction-id`, `--body` JSON schema constraints,
//! and offline `--dry-run` request verification).

use crate::daemon::scrub_secrets;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Default subprocess wall-clock timeout for `gemini-api` invocations (`120 s`).
pub const DEFAULT_CLI_TIMEOUT_SEC: u64 = 120;
/// Default maximum stdout/stderr capture size (`16 MiB`) preventing unbounded OOM.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Default general-purpose Gemini 4 model (`gemini-api-cli` default).
pub const GEMINI_4_HYDROGEN: &str = "gemini-4-hydrogen";
/// Frontier-level intelligence with fast end-to-end completion.
pub const GEMINI_4_NEON: &str = "gemini-4-neon";
/// Hardest reasoning and coding model.
pub const GEMINI_4_ARGON: &str = "gemini-4-argon";
/// Alternative hardest reasoning model.
pub const GEMINI_4_CARBON: &str = "gemini-4-carbon";
/// High-throughput lightweight model for Tier-0 likelihood ticks.
pub const GEMINI_3_5_FLASH_LITE: &str = "gemini-3.5-flash-lite";
/// Audio transcription model (`24 min` chunks, `§15`).
pub const GEMINI_3_5_TRANSCRIBE: &str = "gemini-3.5-transcribe";
/// Managed Deep Research agent (`--background`).
pub const DEEP_RESEARCH_AGENT: &str = "deep-research-preview-04-2026";
/// Managed Antigravity remote sandbox agent.
pub const ANTIGRAVITY_AGENT: &str = "antigravity-preview-09-2026";

#[derive(Debug, Error)]
pub enum GeminiCliError {
    #[error(
        "`gemini-api` binary not found at `{binary}`: install from https://github.com/google-gemini/gemini-api-cli"
    )]
    BinaryNotFound { binary: String },
    #[error("failed to spawn or communicate with `gemini-api`: {0}")]
    Io(#[from] std::io::Error),
    #[error("`gemini-api` authentication error (exit 3): {stderr}")]
    AuthRequired { stderr: String },
    #[error("`gemini-api` usage/validation error (exit 2): {stderr}")]
    UsageValidation { stderr: String },
    #[error("`gemini-api` runtime/API error (exit {code}): {stderr}")]
    RuntimeApi { code: i32, stderr: String },
    #[error("`gemini-api` subprocess timed out after {timeout_sec}s and was terminated")]
    TimedOut { timeout_sec: u64 },
    #[error("`gemini-api` subprocess output exceeded hard cap ({cap_bytes} bytes)")]
    OutputTooLarge { cap_bytes: usize },
    #[error("failed to parse `gemini-api` JSON output: {0}")]
    Json(#[from] serde_json::Error),
}

/// Configuration for invoking the `gemini-api` CLI.
#[derive(Debug, Clone)]
pub struct GeminiCliConfig {
    /// Path or binary name (defaults to `GEMINI_API_BIN` env var or `"gemini-api"`).
    pub binary: PathBuf,
    /// Optional explicit API key override (passed via `GEMINI_API_KEY` env, never CLI args).
    pub api_key: Option<String>,
    /// Optional custom server URL (`--server-url`).
    pub server_url: Option<String>,
    /// Optional `Api-Revision` override (e.g., `2026-05-20`).
    pub api_revision: Option<String>,
    /// Optional custom `HOME` directory (useful for isolated/sandboxed dry-run tests).
    pub home_override: Option<PathBuf>,
    /// Disable OS keyring lookup (`GEMINI_NO_KEYRING=1`).
    pub no_keyring: bool,
    /// Hard wall-clock timeout in seconds (`DEFAULT_CLI_TIMEOUT_SEC`).
    pub timeout_sec: u64,
    /// Hard byte cap on captured stdout/stderr (`DEFAULT_MAX_OUTPUT_BYTES`).
    pub max_output_bytes: usize,
}

impl Default for GeminiCliConfig {
    fn default() -> Self {
        let binary = std::env::var_os("GEMINI_API_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("gemini-api"));
        Self {
            binary,
            api_key: None,
            server_url: None,
            api_revision: None,
            home_override: None,
            no_keyring: false,
            timeout_sec: DEFAULT_CLI_TIMEOUT_SEC,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

/// Parsed credential source entry from `gemini-api whoami -o json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialSourceInfo {
    pub source: String,
    pub value: String,
}

/// Parsed `gemini-api whoami -o json` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiWhoami {
    pub config_file: Option<String>,
    pub api_key: CredentialSourceInfo,
    pub access_token: CredentialSourceInfo,
    pub api_version: Option<String>,
}

/// Request parameters for `gemini-api generate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiGenerateRequest {
    pub model: String,
    pub prompt: String,
    pub system_instruction: Option<String>,
    /// Server-side multi-turn chain handle (used for staged-reveal Turn A -> Turn B).
    pub previous_interaction_id: Option<String>,
    /// Local file paths, `files/<id>` URIs, or YouTube URLs passed via repeatable `-i`.
    pub inputs: Vec<String>,
    /// Optional raw JSON `--body` (for `response_format` JSON schema, `tools`, `store`, etc.).
    pub body_json: Option<serde_json::Value>,
    /// If true, passes `--dry-run` to validate and preview the redacted HTTP request offline.
    pub dry_run: bool,
}

impl GeminiGenerateRequest {
    #[must_use]
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
            system_instruction: None,
            previous_interaction_id: None,
            inputs: Vec::new(),
            body_json: None,
            dry_run: false,
        }
    }
}

/// Request parameters for `gemini-api analyze`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiAnalyzeRequest {
    pub model: String,
    pub question: String,
    pub inputs: Vec<String>,
    pub system_instruction: Option<String>,
    pub dry_run: bool,
}

/// Token and estimated USD cost usage parsed from a Gemini Interactions response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeminiUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

/// Normalized response returned by `gemini-api generate`, `analyze`, or `agent status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiGenerateResponse {
    pub id: Option<String>,
    pub status: Option<String>,
    pub model: String,
    pub text: String,
    pub dry_run: bool,
    pub usage: Option<GeminiUsage>,
    pub raw_json: serde_json::Value,
}

/// Response from launching a background managed agent (`gemini-api agent run --background`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiAgentRun {
    pub id: String,
    pub status: Option<String>,
    pub environment_id: Option<String>,
    pub raw_json: serde_json::Value,
}

/// Pure-Rust subprocess client wrapping `google-gemini/gemini-api-cli`.
#[derive(Debug, Clone)]
pub struct GeminiCliClient {
    config: GeminiCliConfig,
}

impl GeminiCliClient {
    #[must_use]
    pub fn new(config: GeminiCliConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn from_env() -> Self {
        Self::new(GeminiCliConfig::default())
    }

    #[must_use]
    pub fn config(&self) -> &GeminiCliConfig {
        &self.config
    }

    /// Builds the exact argument vector for `gemini-api generate` (prompt delivered via `--stdin`).
    #[must_use]
    pub fn build_generate_args(&self, req: &GeminiGenerateRequest) -> Vec<String> {
        let mut args = vec![
            "generate".to_owned(),
            "-m".to_owned(),
            req.model.clone(),
            "--stdin".to_owned(),
            "--stream=false".to_owned(),
            "--no-interactive".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ];
        if let Some(sys) = &req.system_instruction {
            args.push("--system".to_owned());
            args.push(sys.clone());
        }
        if let Some(prev) = &req.previous_interaction_id {
            args.push("--previous-interaction-id".to_owned());
            args.push(prev.clone());
        }
        for input in &req.inputs {
            args.push("-i".to_owned());
            args.push(input.clone());
        }
        if let Some(body) = &req.body_json {
            args.push("--body".to_owned());
            args.push(body.to_string());
        }
        if req.dry_run {
            args.push("--dry-run".to_owned());
        }
        self.push_global_flags(&mut args);
        args
    }

    /// Checks active Gemini credentials and winning auth source (`gemini-api whoami -o json`).
    pub fn whoami(&self) -> Result<GeminiWhoami, GeminiCliError> {
        let mut args = vec![
            "whoami".to_owned(),
            "--no-interactive".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ];
        self.push_global_flags(&mut args);
        let raw = self.run_json(&args, None)?;

        let creds = raw.get("credentials");
        let parse_cred = |key: &str| -> CredentialSourceInfo {
            let entry = creds.and_then(|c| c.get(key));
            CredentialSourceInfo {
                source: entry
                    .and_then(|e| e.get("source"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unset")
                    .to_owned(),
                value: entry
                    .and_then(|e| e.get("value"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("(not set)")
                    .to_owned(),
            }
        };

        Ok(GeminiWhoami {
            config_file: raw
                .get("config_file")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            api_key: parse_cred("api-key"),
            access_token: parse_cred("access-token"),
            api_version: raw
                .get("global_parameters")
                .and_then(|g| g.get("api-version"))
                .and_then(|v| v.get("value"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        })
    }

    /// Runs `gemini-api generate` with `--stream=false -o json` (or `--dry-run`).
    pub fn generate(
        &self,
        req: &GeminiGenerateRequest,
    ) -> Result<GeminiGenerateResponse, GeminiCliError> {
        let args = self.build_generate_args(req);
        let raw = self.run_json(&args, Some(req.prompt.as_bytes()))?;
        Ok(parse_generate_response(&req.model, raw))
    }

    /// Runs `gemini-api analyze` over one or more `-i` local files, `files/<id>` URIs, or URLs.
    pub fn analyze(
        &self,
        req: &GeminiAnalyzeRequest,
    ) -> Result<GeminiGenerateResponse, GeminiCliError> {
        let mut args = vec![
            "analyze".to_owned(),
            "-m".to_owned(),
            req.model.clone(),
            "--stdin".to_owned(),
            "--no-interactive".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ];
        if let Some(sys) = &req.system_instruction {
            args.push("--system".to_owned());
            args.push(sys.clone());
        }
        for input in &req.inputs {
            args.push("-i".to_owned());
            args.push(input.clone());
        }
        if req.dry_run {
            args.push("--dry-run".to_owned());
        }
        self.push_global_flags(&mut args);
        let raw = self.run_json(&args, Some(req.question.as_bytes()))?;
        Ok(parse_generate_response(&req.model, raw))
    }

    /// Launches a background managed agent (`deep-research-preview-04-2026`, `antigravity-preview-09-2026`),
    /// delivering the prompt JSON body over stdin (`--body @-`) so prompts never appear in `argv`/`ps`
    /// or hit Linux's 128 KB per-argument ceiling.
    pub fn agent_run_background(
        &self,
        agent_id: &str,
        prompt: &str,
        body_json: Option<&serde_json::Value>,
        dry_run: bool,
    ) -> Result<GeminiAgentRun, GeminiCliError> {
        if agent_id.is_empty() || agent_id.starts_with('-') {
            return Err(GeminiCliError::UsageValidation {
                stderr: format!("invalid agent_id: {agent_id:?}"),
            });
        }
        let mut payload = match body_json {
            Some(serde_json::Value::Object(map)) => map.clone(),
            Some(_) => {
                return Err(GeminiCliError::UsageValidation {
                    stderr: "body_json must be a JSON object".into(),
                });
            }
            None => serde_json::Map::new(),
        };
        payload.insert(
            "agent".to_owned(),
            serde_json::Value::String(agent_id.to_owned()),
        );
        payload.insert(
            "input".to_owned(),
            serde_json::Value::String(prompt.to_owned()),
        );
        payload.insert("background".to_owned(), serde_json::Value::Bool(true));
        let body_bytes = serde_json::Value::Object(payload).to_string();

        let mut args = vec![
            "agent".to_owned(),
            "run".to_owned(),
            "--agent".to_owned(),
            agent_id.to_owned(),
            "--background".to_owned(),
            "--body".to_owned(),
            "@-".to_owned(),
            "--no-interactive".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ];
        if dry_run {
            args.push("--dry-run".to_owned());
        }
        self.push_global_flags(&mut args);
        let raw = self.run_json(&args, Some(body_bytes.as_bytes()))?;
        let id = raw
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("dry-run")
            .to_owned();
        let status = raw
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let environment_id = raw
            .get("environment_id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        Ok(GeminiAgentRun {
            id,
            status,
            environment_id,
            raw_json: raw,
        })
    }

    /// Polls a managed agent run via `gemini-api agent status <run_id> -o json`.
    pub fn agent_status(&self, run_id: &str) -> Result<GeminiGenerateResponse, GeminiCliError> {
        if run_id.is_empty() || run_id.starts_with('-') || run_id.chars().any(char::is_whitespace) {
            return Err(GeminiCliError::UsageValidation {
                stderr: format!("invalid run_id: {run_id:?}"),
            });
        }
        let mut args = vec![
            "agent".to_owned(),
            "status".to_owned(),
            "--no-interactive".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ];
        self.push_global_flags(&mut args);
        args.push("--".to_owned());
        args.push(run_id.to_owned());
        let raw = self.run_json(&args, None)?;
        Ok(parse_generate_response("agent", raw))
    }

    /// Queries the live Gemini model catalog (`gemini-api models list -o json`, backs `fcast doctor --check-models`).
    pub fn models_list(&self) -> Result<serde_json::Value, GeminiCliError> {
        let mut args = vec![
            "models".to_owned(),
            "list".to_owned(),
            "--no-interactive".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ];
        self.push_global_flags(&mut args);
        self.run_json(&args, None)
    }

    fn push_global_flags(&self, args: &mut Vec<String>) {
        if let Some(url) = &self.config.server_url {
            args.push("--server-url".to_owned());
            args.push(url.clone());
        }
    }

    fn run_json(
        &self,
        args: &[String],
        stdin_bytes: Option<&[u8]>,
    ) -> Result<serde_json::Value, GeminiCliError> {
        let mut cmd = Command::new(&self.config.binary);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if stdin_bytes.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });

        if let Some(key) = &self.config.api_key {
            cmd.env("GEMINI_API_KEY", key);
        }
        if let Some(rev) = &self.config.api_revision {
            cmd.env("GEMINI_API_REVISION", rev);
        }
        if let Some(home) = &self.config.home_override {
            cmd.env("HOME", home);
        }
        if self.config.no_keyring {
            cmd.env("GEMINI_NO_KEYRING", "1");
        }

        let mut child = cmd.spawn().map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                GeminiCliError::BinaryNotFound {
                    binary: self.config.binary.display().to_string(),
                }
            } else {
                GeminiCliError::Io(err)
            }
        })?;

        // Stream stdin and bounded stdout/stderr concurrently while polling `try_wait` against
        // `timeout_sec` so large prompts never deadlock, hung processes are terminated, and
        // oversized output cannot OOM the caller.
        let cap = self.config.max_output_bytes.max(1024);
        let timeout_sec = self.config.timeout_sec.max(1);
        let deadline = Instant::now() + Duration::from_secs(timeout_sec);
        let mut timed_out = false;
        let mut status_opt = None;

        let (stdout_res, stderr_res) = std::thread::scope(|scope| {
            let stdin_handle = match (stdin_bytes, child.stdin.take()) {
                (Some(bytes), Some(mut stdin)) => Some(scope.spawn(move || {
                    if let Err(e) = stdin.write_all(bytes)
                        && e.kind() != std::io::ErrorKind::BrokenPipe
                    {
                        return Err(e);
                    }
                    Ok(())
                })),
                _ => None,
            };
            let stdout_handle = child
                .stdout
                .take()
                .map(|mut out| scope.spawn(move || read_bounded_pipe(&mut out, cap)));
            let stderr_handle = child
                .stderr
                .take()
                .map(|mut err| scope.spawn(move || read_bounded_pipe(&mut err, cap)));

            loop {
                match child.try_wait() {
                    Ok(Some(st)) => {
                        status_opt = Some(st);
                        break;
                    }
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            timed_out = true;
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }

            if let Some(h) = stdin_handle {
                let _ = h.join();
            }
            let out = stdout_handle
                .and_then(|h| h.join().ok())
                .unwrap_or_else(|| Ok(Vec::new()));
            let err = stderr_handle
                .and_then(|h| h.join().ok())
                .unwrap_or_else(|| Ok(Vec::new()));
            (out, err)
        });

        if timed_out {
            return Err(GeminiCliError::TimedOut { timeout_sec });
        }
        let stdout_bytes = stdout_res?;
        let stderr_bytes = stderr_res?;
        let status = status_opt.ok_or_else(|| {
            GeminiCliError::Io(std::io::Error::other("failed to collect child exit status"))
        })?;
        if !status.success() {
            let code = status.code().unwrap_or(1);
            let stderr = scrub_secrets(&String::from_utf8_lossy(&stderr_bytes))
                .trim()
                .to_owned();
            return Err(match code {
                2 => GeminiCliError::UsageValidation { stderr },
                3 => GeminiCliError::AuthRequired { stderr },
                other => GeminiCliError::RuntimeApi {
                    code: other,
                    stderr,
                },
            });
        }

        let stdout = String::from_utf8_lossy(&stdout_bytes);
        Ok(serde_json::from_str(stdout.trim())?)
    }
}

fn read_bounded_pipe<R: Read>(reader: &mut R, cap: usize) -> Result<Vec<u8>, GeminiCliError> {
    let mut buf = Vec::new();
    let mut limited = reader.take((cap as u64).saturating_add(1));
    limited.read_to_end(&mut buf)?;
    if buf.len() > cap {
        return Err(GeminiCliError::OutputTooLarge { cap_bytes: cap });
    }
    Ok(buf)
}

/// Extracts reply text from a Gemini Interactions API JSON response (`gemini-api --stream=false -o json`).
///
/// Follows the `gemini-api` schema: `.steps[] | select(.type=="model_output") | .content[] | select(.type=="text") | .text`,
/// falling back to `.outputs[]` if present.
#[must_use]
pub fn extract_interaction_text(raw: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(steps) = raw.get("steps").and_then(serde_json::Value::as_array) {
        for step in steps {
            if step.get("type").and_then(serde_json::Value::as_str) == Some("model_output")
                && let Some(contents) = step.get("content").and_then(serde_json::Value::as_array)
            {
                for item in contents {
                    if item.get("type").and_then(serde_json::Value::as_str) == Some("text")
                        && let Some(text) = item.get("text").and_then(serde_json::Value::as_str)
                    {
                        parts.push(text);
                    }
                }
            }
        }
    }
    if parts.is_empty()
        && let Some(outputs) = raw.get("outputs").and_then(serde_json::Value::as_array)
    {
        for item in outputs {
            if item.get("type").and_then(serde_json::Value::as_str) == Some("text")
                && let Some(text) = item.get("text").and_then(serde_json::Value::as_str)
            {
                parts.push(text);
            }
        }
    }
    parts.concat()
}

fn parse_generate_response(default_model: &str, raw: serde_json::Value) -> GeminiGenerateResponse {
    let dry_run = raw
        .get("dry_run")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let id = raw
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let status = raw
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let model = raw
        .get("model")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            raw.get("request")
                .and_then(|r| r.get("body"))
                .and_then(|b| b.get("model"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or(default_model)
        .to_owned();
    let text = extract_interaction_text(&raw);
    let usage = parse_usage(&raw);

    GeminiGenerateResponse {
        id,
        status,
        model,
        text,
        dry_run,
        usage,
        raw_json: raw,
    }
}

fn parse_usage(raw: &serde_json::Value) -> Option<GeminiUsage> {
    let u = raw.get("usage").or_else(|| raw.get("usage_metadata"))?;
    let input_tokens = u
        .get("input_tokens")
        .or_else(|| u.get("prompt_token_count"))
        .and_then(serde_json::Value::as_u64);
    let output_tokens = u
        .get("output_tokens")
        .or_else(|| u.get("candidates_token_count"))
        .and_then(serde_json::Value::as_u64);
    let total_tokens = u
        .get("total_tokens")
        .or_else(|| u.get("total_token_count"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| match (input_tokens, output_tokens) {
            (Some(i), Some(o)) => Some(i.saturating_add(o)),
            _ => None,
        });
    let explicit_cost = u
        .get("estimated_cost_usd")
        .or_else(|| u.get("cost_usd"))
        .and_then(serde_json::Value::as_f64)
        .filter(|c| c.is_finite() && *c >= 0.0);
    // Conservative fallback cost estimate when provider returns token counts without USD cost.
    let estimated_cost_usd = explicit_cost.or_else(|| {
        let inp = input_tokens? as f64;
        let out = output_tokens.unwrap_or(0) as f64;
        Some((inp * 1.25e-6) + (out * 5.0e-6))
    });
    Some(GeminiUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        estimated_cost_usd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_flag_injection_in_agent_ids_and_run_ids() {
        let client = GeminiCliClient::from_env();
        assert!(matches!(
            client.agent_status("--help"),
            Err(GeminiCliError::UsageValidation { .. })
        ));
        assert!(matches!(
            client.agent_status("id with spaces"),
            Err(GeminiCliError::UsageValidation { .. })
        ));
        assert!(matches!(
            client.agent_run_background("--evil", "hello", None, true),
            Err(GeminiCliError::UsageValidation { .. })
        ));
    }

    #[test]
    fn parses_usage_and_enforces_subprocess_timeout_and_output_cap() {
        let with_cost = serde_json::json!({
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 200,
                "estimated_cost_usd": 0.042
            }
        });
        let u1 = parse_usage(&with_cost).expect("usage with cost");
        assert_eq!(u1.total_tokens, Some(1200));
        assert!((u1.estimated_cost_usd.expect("cost") - 0.042).abs() < 1e-12);

        let tokens_only = serde_json::json!({
            "usage_metadata": {
                "prompt_token_count": 2000,
                "candidates_token_count": 500
            }
        });
        let u2 = parse_usage(&tokens_only).expect("usage fallback");
        assert_eq!(u2.total_tokens, Some(2500));
        assert!(u2.estimated_cost_usd.expect("fallback cost") > 0.0);

        // Subprocess timeout kills hung process
        let sleeper = GeminiCliClient::new(GeminiCliConfig {
            binary: PathBuf::from("/bin/sleep"),
            timeout_sec: 1,
            ..GeminiCliConfig::default()
        });
        let err = sleeper.run_json(&["10".to_owned()], None);
        assert!(matches!(
            err,
            Err(GeminiCliError::TimedOut { timeout_sec: 1 })
        ));

        // Output cap rejects oversized stdout
        let noisy = GeminiCliClient::new(GeminiCliConfig {
            binary: PathBuf::from("/bin/sh"),
            timeout_sec: 5,
            max_output_bytes: 1024,
            ..GeminiCliConfig::default()
        });
        let err_cap = noisy.run_json(
            &[
                "-c".to_owned(),
                "dd if=/dev/zero bs=1024 count=4 2>/dev/null | tr '\\0' 'a'".to_owned(),
            ],
            None,
        );
        assert!(matches!(
            err_cap,
            Err(GeminiCliError::OutputTooLarge { cap_bytes: 1024 })
        ));
    }
}
