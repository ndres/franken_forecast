//! `Metronome` (`SUB-06` `fcastd`) coordination primitives (`§13`, `§13.2`, `GATE-19`):
//! - `0600`-permissioned `UdsEventBus` over `daemon.sock` (enforcing `<= 104`-byte `sun_path`),
//! - Immutable `QuestionRunContext` & close-derived `TimeBudget` scheduling,
//! - 4-tier `BillingPacingTier` (`Full` / `Economy` / `Minimal` / `Stop`),
//! - `WatchdogAction` (`45 min` alert / `75 min` kill-and-reclaim),
//! - `scrub_secrets` for bearer/OAuth/API token redaction in logs & dead-letter payloads.

use crate::jobs::DomainEventRecord;
use crate::sys::{UdsPathError, validate_uds_sun_path};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

/// Minimum seconds remaining before close/spot-scoring required to admit a question (`§13.2`).
pub const MIN_INTAKE_SEC: i64 = 120;

/// Threshold (seconds remaining) below which `TimeBudget` degrades to single-pass `FastPath` (`§13.2`).
pub const FAST_PATH_THRESHOLD_SEC: i64 = 600;

/// Stalled lease alert threshold (`45 min`, `§13.2`).
pub const WATCHDOG_ALERT_SEC: i64 = 45 * 60;

/// Stalled lease kill-and-reclaim threshold (`75 min`, `§13.2`).
pub const WATCHDOG_KILL_RECLAIM_SEC: i64 = 75 * 60;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("invalid UDS socket path: {0}")]
    InvalidUdsPath(#[from] UdsPathError),
    #[error("daemon I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("UDS connection closed before receiving a complete event frame")]
    UnexpectedEof,
}

/// JSON-line event envelope broadcast over the `0600` `daemon.sock` UDS bus (`§13`, `§13.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonBusEvent {
    pub event_seq: i64,
    pub event_kind: String,
    pub question_id: String,
    pub forecast_id: Option<String>,
    pub as_of_utc: String,
    pub payload_json: String,
}

impl From<&DomainEventRecord> for DaemonBusEvent {
    fn from(rec: &DomainEventRecord) -> Self {
        Self {
            event_seq: rec.event_seq,
            event_kind: rec.event_kind.clone(),
            question_id: rec.question_id.clone(),
            forecast_id: rec.forecast_id.clone(),
            as_of_utc: rec.as_of_utc.clone(),
            payload_json: rec.payload_json.clone(),
        }
    }
}

/// `0600`-permissioned Unix Domain Socket JSON-line event push bus for `daemon.sock` (`§13`).
#[cfg(unix)]
#[derive(Debug)]
pub struct UdsEventBus {
    socket_path: PathBuf,
    listener: UnixListener,
}

#[cfg(unix)]
impl UdsEventBus {
    /// Binds a new `0600`-permissioned Unix Domain Socket listener at `socket_path`
    /// after validating the `<= 104`-byte POSIX `sun_path` ceiling.
    pub fn bind(socket_path: impl AsRef<Path>) -> Result<Self, DaemonError> {
        let socket_path = socket_path.as_ref().to_path_buf();
        validate_uds_sun_path(&socket_path)?;
        if let Some(parent) = socket_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            socket_path,
            listener,
        })
    }

    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<(), DaemonError> {
        self.listener.set_nonblocking(nonblocking)?;
        Ok(())
    }

    pub fn accept(&self) -> Result<UnixStream, DaemonError> {
        let (stream, _addr) = self.listener.accept()?;
        Ok(stream)
    }

    /// Connects a client `UnixStream` to a bound `daemon.sock` path.
    pub fn connect(socket_path: impl AsRef<Path>) -> Result<UnixStream, DaemonError> {
        let socket_path = socket_path.as_ref();
        validate_uds_sun_path(socket_path)?;
        Ok(UnixStream::connect(socket_path)?)
    }

    /// Pushes one newline-delimited JSON [`DaemonBusEvent`] over `stream`.
    pub fn push_event(stream: &mut UnixStream, event: &DaemonBusEvent) -> Result<(), DaemonError> {
        let line = serde_json::to_string(event)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        Ok(())
    }

    /// Receives one newline-delimited JSON [`DaemonBusEvent`] from `stream`.
    pub fn recv_event(stream: &mut UnixStream) -> Result<DaemonBusEvent, DaemonError> {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Err(DaemonError::UnexpectedEof);
        }
        Ok(serde_json::from_str(line.trim_end())?)
    }
}

#[cfg(unix)]
impl Drop for UdsEventBus {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

/// Close-derived execution mode computed by [`TimeBudget`] (`§13.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeBudgetMode {
    /// `now_utc >= spot_scoring_utc`: skip immediately (`$0` spend).
    SkipAfterSpotScoring,
    /// `remaining < 120 s`: drop question at intake (`excluded`).
    IntakeSkip,
    /// `120 s <= remaining < 600 s`: degrade to single-pass fast synthesis.
    FastPath,
    /// `remaining >= 600 s`: full multi-stage epistemic pipeline.
    Full,
}

/// Close-derived stage budget derived from `min(spot_scoring_utc, close_utc) - now_utc` (`§13.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeBudget {
    pub remaining_sec: i64,
    pub effective_deadline_epoch: i64,
    pub skip_after_spot_scoring_time: bool,
    pub mode: TimeBudgetMode,
}

impl TimeBudget {
    /// Computes close-derived scheduling budget from Unix epoch timestamps (seconds).
    #[must_use]
    pub fn from_epochs(now_epoch: i64, spot_scoring_epoch: Option<i64>, close_epoch: i64) -> Self {
        let effective_deadline_epoch =
            spot_scoring_epoch.map_or(close_epoch, |spot| spot.min(close_epoch));
        if let Some(spot) = spot_scoring_epoch
            && now_epoch >= spot
        {
            return Self {
                remaining_sec: (effective_deadline_epoch - now_epoch).max(0),
                effective_deadline_epoch,
                skip_after_spot_scoring_time: true,
                mode: TimeBudgetMode::SkipAfterSpotScoring,
            };
        }
        let remaining_sec = effective_deadline_epoch - now_epoch;
        let mode = if remaining_sec < MIN_INTAKE_SEC {
            TimeBudgetMode::IntakeSkip
        } else if remaining_sec < FAST_PATH_THRESHOLD_SEC {
            TimeBudgetMode::FastPath
        } else {
            TimeBudgetMode::Full
        };
        Self {
            remaining_sec,
            effective_deadline_epoch,
            skip_after_spot_scoring_time: false,
            mode,
        }
    }

    /// Returns `true` if the question should be skipped without spending LLM budget.
    #[must_use]
    pub const fn should_skip(&self) -> bool {
        matches!(
            self.mode,
            TimeBudgetMode::SkipAfterSpotScoring | TimeBudgetMode::IntakeSkip
        )
    }
}

/// 4-tier billing-cycle pacing policy (`§13`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BillingPacingTier {
    /// `< 70%` of billing-cycle budget spent.
    Full,
    /// `70% .. 90%` spent: single research report + reduced panel.
    Economy,
    /// `90% .. 100%` spent: zero-key search + single fast model + `THIN_PUBLISH_FLOOR`.
    Minimal,
    /// `>= 100%` spent (or non-positive/invalid budget): hard halt.
    Stop,
}

impl BillingPacingTier {
    /// Evaluates the pacing tier from `(spent_usd, budget_usd)`.
    #[must_use]
    pub fn from_spend(spent_usd: f64, budget_usd: f64) -> Self {
        if !spent_usd.is_finite() || !budget_usd.is_finite() || budget_usd <= 0.0 {
            return Self::Stop;
        }
        let ratio = (spent_usd / budget_usd).max(0.0);
        if ratio >= 1.0 {
            Self::Stop
        } else if ratio >= 0.90 {
            Self::Minimal
        } else if ratio >= 0.70 {
            Self::Economy
        } else {
            Self::Full
        }
    }

    #[must_use]
    pub const fn allows_dispatch(self) -> bool {
        !matches!(self, Self::Stop)
    }
}

/// `Control / Control / Challenger` A/B slot assignment (`§13.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AbSlot {
    ControlA,
    ControlB,
    Challenger,
}

/// Shared immutable coordination spine passed across `Harvester -> Notary -> Loom -> Crucible` (`§13.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionRunContext {
    pub run_id: String,
    pub question_id: String,
    pub platform_id: String,
    pub tournament_id: Option<String>,
    pub as_of_utc: String,
    pub spot_scoring_utc: Option<String>,
    pub close_utc: String,
    pub time_budget: TimeBudget,
    pub pacing_tier: BillingPacingTier,
    pub ab_slot: AbSlot,
    pub prompt_cache_prefix_hash: String,
    pub donated_credit_pool: Option<String>,
}

/// Stalled-worker watchdog decision (`§13.2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WatchdogAction {
    /// Stalled duration `< 45 min`.
    Healthy,
    /// Stalled duration in `[45 min, 75 min)`: emit `WorkerStallDetected` alert.
    AlertAt45Min,
    /// Stalled duration `>= 75 min`: kill hung worker and reclaim lease.
    KillAndReclaimAt75Min,
}

impl WatchdogAction {
    /// Evaluates the watchdog action from elapsed stalled seconds.
    #[must_use]
    pub const fn evaluate(stalled_sec: i64) -> Self {
        if stalled_sec >= WATCHDOG_KILL_RECLAIM_SEC {
            Self::KillAndReclaimAt75Min
        } else if stalled_sec >= WATCHDOG_ALERT_SEC {
            Self::AlertAt45Min
        } else {
            Self::Healthy
        }
    }
}

const REDACTED: &str = "[REDACTED]";

/// Redacts bearer, OAuth, and API tokens from structured logs, dead-letter payloads,
/// and crash receipts before persistence (`§13`).
#[must_use]
pub fn scrub_secrets(input: &str) -> String {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        // 1. Authorization header: "Authorization: Bearer <tok>" or "Authorization: <tok>"
        if let Some(prefix_len) = match_ascii_ci(bytes, i, b"authorization:") {
            let mut j = i + prefix_len;
            while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            out.push_str(&input[i..j]);
            if let Some(bearer_len) = match_ascii_ci(bytes, j, b"bearer ") {
                out.push_str(&input[j..j + bearer_len]);
                j += bearer_len;
                while j < len && bytes[j] == b' ' {
                    out.push(' ');
                    j += 1;
                }
            }
            let end = scan_token_end(bytes, j);
            out.push_str(REDACTED);
            i = end;
            continue;
        }

        // 2. Standalone "Bearer <token>"
        if is_boundary_before(bytes, i)
            && let Some(bearer_len) = match_ascii_ci(bytes, i, b"bearer ")
        {
            let mut j = i + bearer_len;
            while j < len && bytes[j] == b' ' {
                j += 1;
            }
            let end = scan_token_end(bytes, j);
            if end > j {
                out.push_str(&input[i..j]);
                out.push_str(REDACTED);
                i = end;
                continue;
            }
        }

        // 3. Key-value secret fields: api_key, access_token, refresh_token, oauth_token, client_secret, x-api-key, token
        if is_boundary_before(bytes, i)
            && let Some((val_start, val_end)) = match_kv_secret(bytes, i)
        {
            out.push_str(&input[i..val_start]);
            out.push_str(REDACTED);
            i = val_end;
            continue;
        }

        // 4. Known provider API / OAuth key prefixes
        if is_boundary_before(bytes, i) && has_known_token_prefix(bytes, i) {
            let end = scan_token_end(bytes, i);
            if end - i >= 12 {
                out.push_str(REDACTED);
                i = end;
                continue;
            }
        }

        // Advance one UTF-8 char
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn match_ascii_ci(haystack: &[u8], pos: usize, needle_lower: &[u8]) -> Option<usize> {
    let n = needle_lower.len();
    if pos + n > haystack.len() {
        return None;
    }
    for k in 0..n {
        if haystack[pos + k].to_ascii_lowercase() != needle_lower[k] {
            return None;
        }
    }
    Some(n)
}

fn is_boundary_before(bytes: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let prev = bytes[pos - 1];
    !prev.is_ascii_alphanumeric() && prev != b'_' && prev != b'-'
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+' | b'/' | b'=' | b'~')
}

fn scan_token_end(bytes: &[u8], start: usize) -> usize {
    let mut j = start;
    while j < bytes.len() && is_token_char(bytes[j]) {
        j += 1;
    }
    j
}

fn has_known_token_prefix(bytes: &[u8], pos: usize) -> bool {
    const PREFIXES: &[&[u8]] = &[
        b"sk-ant-",
        b"sk-proj-",
        b"sk-or-",
        b"sk-",
        b"AIza",
        b"ya29.",
        b"ghp_",
        b"gho_",
        b"ghu_",
        b"ghs_",
        b"ghr_",
        b"xoxb-",
        b"xoxp-",
        b"xoxa-",
        b"xoxr-",
    ];
    PREFIXES
        .iter()
        .any(|p| pos + p.len() <= bytes.len() && &bytes[pos..pos + p.len()] == *p)
}

fn match_kv_secret(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    const KEYS: &[&[u8]] = &[
        b"x-api-key",
        b"api_key",
        b"apikey",
        b"access_token",
        b"refresh_token",
        b"oauth_token",
        b"client_secret",
        b"bearer_token",
        b"auth_token",
    ];
    let len = bytes.len();
    for key in KEYS {
        if let Some(klen) = match_ascii_ci(bytes, pos, key) {
            let mut j = pos + klen;
            // Optional closing quote for JSON keys like "api_key": "..."
            if j < len && (bytes[j] == b'"' || bytes[j] == b'\'') {
                j += 1;
            }
            while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j >= len || (bytes[j] != b'=' && bytes[j] != b':') {
                continue;
            }
            j += 1;
            while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j < len && (bytes[j] == b'"' || bytes[j] == b'\'') {
                j += 1;
            }
            let val_start = j;
            let val_end = scan_token_end(bytes, val_start);
            if val_end > val_start {
                return Some((val_start, val_end));
            }
        }
    }
    None
}
