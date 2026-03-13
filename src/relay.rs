//! Worker-to-user relay mechanism.
//!
//! Workers that need credentials, keys, or other user input call
//! `superharness relay` to queue a request.  The orchestrator (or watch loop)
//! detects pending relays and forwards them to the human.  Once the human
//! provides an answer via `superharness relay-answer`, the worker polls with
//! `superharness relay --wait-for <id>` to retrieve it.
//!
//! All relay state lives in `~/.local/share/superharness/relay_requests.json`.
//!
//! ## Sudo support
//!
//! `relay_sudo` is a convenience wrapper that creates a sensitive relay
//! request asking for the user's sudo password, then constructs
//! `echo <password> | sudo -S <command>` once the answer arrives.
//! `sudo_exec` tries direct sudo (NOPASSWD path) first and falls back to
//! the relay mechanism if sudo prompts for a password.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Relay request schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RelayStatus {
    Pending,
    Answered,
    Cancelled,
}

impl std::fmt::Display for RelayStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayStatus::Pending => write!(f, "pending"),
            RelayStatus::Answered => write!(f, "answered"),
            RelayStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RelayKind {
    /// Regular question / credential request.
    Question,
    /// Sudo password relay — answer is used to run a command via `sudo -S`.
    Sudo,
}

impl std::fmt::Display for RelayKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayKind::Question => write!(f, "question"),
            RelayKind::Sudo => write!(f, "sudo"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayRequest {
    /// Unique request ID (e.g. "relay-a1b2c3d4").
    pub id: String,
    /// Pane that originated the request.
    pub pane_id: String,
    /// Human-readable question.
    pub question: String,
    /// Additional context to help the human understand why this is needed.
    pub context: String,
    /// If true, the answer is sensitive (password, key, token) and should not
    /// be echoed in logs.
    pub sensitive: bool,
    /// Request type.
    pub kind: RelayKind,
    /// For sudo relays: the command that needs to run with elevated privileges.
    pub sudo_command: Option<String>,
    pub status: RelayStatus,
    /// The answer provided by the human (None until answered).
    pub answer: Option<String>,
    pub created_at: u64,
    pub answered_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn relay_file() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("cannot determine home directory")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
        .join("relay_requests.json"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn generate_id() -> String {
    // Use low-precision timestamp + pseudo-random bytes from /dev/urandom
    let ts = now_unix();
    let rand_bytes: Vec<u8> = fs::read("/dev/urandom")
        .unwrap_or_default()
        .into_iter()
        .take(4)
        .collect();
    let hex: String = rand_bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("relay-{ts:x}{hex}")
}

fn load_all() -> Result<Vec<RelayRequest>> {
    let path = relay_file()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read relay file: {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let requests: Vec<RelayRequest> = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse relay file: {}", path.display()))?;
    Ok(requests)
}

fn save_all(requests: &[RelayRequest]) -> Result<()> {
    let path = relay_file()?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create relay directory: {}", dir.display()))?;
    }
    let content = serde_json::to_string_pretty(requests).context("failed to serialize relays")?;
    fs::write(&path, content)
        .with_context(|| format!("failed to write relay file: {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create and persist a new relay request.  Returns the request ID.
pub fn add_relay_request(
    pane_id: &str,
    question: &str,
    context: &str,
    sensitive: bool,
) -> Result<String> {
    let id = generate_id();
    let mut requests = load_all()?;
    requests.push(RelayRequest {
        id: id.clone(),
        pane_id: pane_id.to_string(),
        question: question.to_string(),
        context: context.to_string(),
        sensitive,
        kind: RelayKind::Question,
        sudo_command: None,
        status: RelayStatus::Pending,
        answer: None,
        created_at: now_unix(),
        answered_at: None,
    });
    save_all(&requests)?;
    Ok(id)
}

/// Create and persist a sudo relay request.  Returns the request ID.
pub fn relay_sudo(pane_id: &str, command: &str) -> Result<String> {
    let id = generate_id();
    let question = format!(
        "Worker pane {pane_id} needs to run: sudo {command}\nPlease enter your sudo password"
    );
    let context = format!(
        "This password will be piped to `sudo -S {command}` in pane {pane_id}. \
        It is transmitted only through superharness local state and is never logged."
    );
    let mut requests = load_all()?;
    requests.push(RelayRequest {
        id: id.clone(),
        pane_id: pane_id.to_string(),
        question,
        context,
        sensitive: true,
        kind: RelayKind::Sudo,
        sudo_command: Some(command.to_string()),
        status: RelayStatus::Pending,
        answer: None,
        created_at: now_unix(),
        answered_at: None,
    });
    save_all(&requests)?;
    Ok(id)
}

/// Return all requests with status == Pending.
pub fn get_pending_relays() -> Result<Vec<RelayRequest>> {
    let all = load_all()?;
    Ok(all
        .into_iter()
        .filter(|r| r.status == RelayStatus::Pending)
        .collect())
}

/// Return all relay requests for a specific pane (all statuses).
pub fn get_relays_for_pane(pane_id: &str) -> Result<Vec<RelayRequest>> {
    let all = load_all()?;
    Ok(all.into_iter().filter(|r| r.pane_id == pane_id).collect())
}

/// Record the human's answer for a relay request, marking it answered.
pub fn answer_relay(request_id: &str, answer: &str) -> Result<()> {
    let mut requests = load_all()?;
    let req = requests
        .iter_mut()
        .find(|r| r.id == request_id)
        .with_context(|| format!("relay request not found: {request_id}"))?;

    if req.status != RelayStatus::Pending {
        bail!("relay request {request_id} is already {:?}", req.status);
    }

    req.status = RelayStatus::Answered;
    req.answer = Some(answer.to_string());
    req.answered_at = Some(now_unix());
    save_all(&requests)?;
    Ok(())
}

/// Cancel a pending relay request.
pub fn cancel_relay(request_id: &str) -> Result<()> {
    let mut requests = load_all()?;
    let req = requests
        .iter_mut()
        .find(|r| r.id == request_id)
        .with_context(|| format!("relay request not found: {request_id}"))?;
    req.status = RelayStatus::Cancelled;
    save_all(&requests)?;
    Ok(())
}

/// Return the answer for a relay request, or None if not yet answered.
pub fn get_answer(request_id: &str) -> Result<Option<String>> {
    let requests = load_all()?;
    let req = requests
        .iter()
        .find(|r| r.id == request_id)
        .with_context(|| format!("relay request not found: {request_id}"))?;
    if req.status == RelayStatus::Answered {
        Ok(req.answer.clone())
    } else {
        Ok(None)
    }
}

/// Block until the relay request is answered (or timeout expires).
/// Polls every 5 seconds.  Returns Some(answer) or None on timeout.
pub fn wait_for_answer(request_id: &str, timeout_secs: u64) -> Result<Option<String>> {
    let deadline = now_unix() + timeout_secs;
    loop {
        if let Some(answer) = get_answer(request_id)? {
            return Ok(Some(answer));
        }
        let remaining = deadline.saturating_sub(now_unix());
        if remaining == 0 {
            return Ok(None);
        }
        let sleep = remaining.min(5);
        std::thread::sleep(Duration::from_secs(sleep));
    }
}

/// Return all relay requests (for relay-list).
pub fn list_all() -> Result<Vec<RelayRequest>> {
    load_all()
}

// ---------------------------------------------------------------------------
// Sudo execution helpers
// ---------------------------------------------------------------------------

/// Run `cmd` directly with `sudo` via a shell one-liner.
/// Returns Ok(()) if the command exits successfully.
/// Returns Err if the command fails or exits non-zero.
pub fn run_sudo_direct(cmd: &str) -> Result<std::process::ExitStatus> {
    let status = std::process::Command::new("sudo")
        .args(["sh", "-c", cmd])
        .status()
        .with_context(|| format!("failed to spawn sudo for: {cmd}"))?;
    Ok(status)
}

/// Run `cmd` via `echo <password> | sudo -S sh -c <cmd>`.
/// Used after the password has been supplied through the relay mechanism.
pub fn run_sudo_with_password(cmd: &str, password: &str) -> Result<std::process::ExitStatus> {
    // Pipe the password on stdin via a shell pipeline.
    let full_cmd = format!(
        "printf '%s\\n' {pw} | sudo -S sh -c {cmd}",
        pw = shell_escape(password),
        cmd = shell_escape(cmd)
    );
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&full_cmd)
        .status()
        .with_context(|| format!("failed to run sudo pipeline for: {cmd}"))?;
    Ok(status)
}

/// Minimal shell-escaping: wrap value in single quotes and escape any
/// existing single-quote characters with '\''
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Sudo-exec: try direct first, relay on password prompt
// ---------------------------------------------------------------------------

/// Attempt to run `cmd` with sudo.  If sudo prompts for a password (detected
/// by examining stderr for "password for"), create a relay request and
/// return its ID so the caller can wait for the human to supply the password.
///
/// Returns either:
///  - `SudoExecResult::Success`   — ran without needing a password
///  - `SudoExecResult::RelayCreated(id)` — password needed; relay request created
///  - `SudoExecResult::Failed(msg)` — command failed for another reason
pub enum SudoExecResult {
    Success,
    RelayCreated(String),
    Failed(String),
}

pub fn sudo_exec(pane_id: &str, cmd: &str) -> Result<SudoExecResult> {
    // Try a non-interactive sudo first (will fail immediately if password needed).
    let output = std::process::Command::new("sudo")
        .args(["-n", "sh", "-c", cmd])
        .output()
        .with_context(|| format!("failed to spawn sudo -n for: {cmd}"))?;

    if output.status.success() {
        return Ok(SudoExecResult::Success);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    let needs_password = stderr.contains("password")
        || stderr.contains("sudo: a password is required")
        || stderr.contains("authentication is required");

    if needs_password {
        let relay_id = relay_sudo(pane_id, cmd)?;
        return Ok(SudoExecResult::RelayCreated(relay_id));
    }

    Ok(SudoExecResult::Failed(
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}
