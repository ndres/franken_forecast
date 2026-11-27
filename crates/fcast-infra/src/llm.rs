//! `LlmRouter` Gemini API adapter backed by the official `google-gemini/gemini-api-cli`
//! (`gemini-api` binary, `§15`, `Plan v2.1`).
//!
//! Keeps `fcast-infra` 100% pure Rust with zero banned HTTP/async crates while giving `fcast`
//! full access to the Gemini Interactions API (`generate`, `analyze`, `agent run/status`,
//! `models list`, `whoami`, `--previous-interaction-id`, `--body` JSON schema constraints,
//! and offline `--dry-run` request verification).

use crate::daemon::scrub_secrets;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use thiserror::Error;

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

/// Normalized response returned by `gemini-api generate`, `analyze`, or `agent status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiGenerateResponse {
    pub id: Option<String>,
    pub status: Option<String>,
    pub model: String,
    pub text: String,
    pub dry_run: bool,
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

    /// Launches a background managed agent (`deep-research-preview-04-2026`, `antigravity-preview-09-2026`).
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
        let mut args = vec![
            "agent".to_owned(),
            "run".to_owned(),
            "--agent".to_owned(),
            agent_id.to_owned(),
            "--background".to_owned(),
            "--no-interactive".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ];
        if let Some(body) = body_json {
            args.push("--body".to_owned());
            args.push(body.to_string());
        }
        if dry_run {
            args.push("--dry-run".to_owned());
        }
        self.push_global_flags(&mut args);
        args.push("--".to_owned());
        args.push(prompt.to_owned());
        let raw = self.run_json(&args, None)?;
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

        // Stream stdin on a scoped thread while draining stdout/stderr so large prompts never
        // deadlock OS pipe buffers and early child exits (e.g., auth failure -> EPIPE) still
        // surface their structured stderr & exit code.
        let output = std::thread::scope(|scope| -> Result<std::process::Output, std::io::Error> {
            let writer_handle = match (stdin_bytes, child.stdin.take()) {
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
            let output = child.wait_with_output()?;
            if let Some(handle) = writer_handle
                && let Ok(write_res) = handle.join()
            {
                write_res?;
            }
            Ok(output)
        })?;
        if !output.status.success() {
            let code = output.status.code().unwrap_or(1);
            let stderr = scrub_secrets(&String::from_utf8_lossy(&output.stderr))
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

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(serde_json::from_str(stdout.trim())?)
    }
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

    GeminiGenerateResponse {
        id,
        status,
        model,
        text,
        dry_run,
        raw_json: raw,
    }
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
}
