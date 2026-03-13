//! Heartbeat and worker status utilities for SuperHarness.
//!
//! This module provides:
//! - `heartbeat()` — sends [HEARTBEAT] messages to the orchestrator pane (%0)
//! - Heartbeat state persistence (read/write to disk)
//! - `status_counts()` — lightweight active/total worker count for the status bar

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::health::{classify_pane, HealthStatus};
use crate::monitor::load_state;
use crate::project;
use crate::tmux;

// ---------------------------------------------------------------------------
// Heartbeat — unconditional periodic message to keep %0 active
// ---------------------------------------------------------------------------

/// Return the local wall-clock time as "HH:MM".
/// Falls back to UTC derived from the Unix timestamp if the `date` command fails.
fn time_hhmm() -> String {
    std::process::Command::new("date")
        .arg("+%H:%M")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let secs_in_day = ts % 86400;
            let h = secs_in_day / 3600;
            let m = (secs_in_day % 3600) / 60;
            format!("{h:02}:{m:02}UTC")
        })
}

/// Return `true` when the last few lines of %0's output suggest it is
/// actively processing (spinner characters visible in multiple recent lines).
/// When %0 is busy, the heartbeat is skipped to avoid interrupting it.
///
/// Requires at least 3 spinner chars across the last 5 lines to trigger —
/// a single spinner char is not enough (avoids over-skipping on static output).
fn orchestrator_is_busy() -> bool {
    let output = match tmux::read("%0", 5) {
        Ok(o) => o,
        Err(_) => return false,
    };

    // Spinner characters used by opencode / claude / shell progress indicators
    let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    // Count total spinner chars across all recent lines.
    // Require at least 3 to reduce false-positives from stale output.
    let spinner_count: usize = output
        .lines()
        .map(|line| spinner_chars.iter().filter(|&&c| line.contains(c)).count())
        .sum();

    spinner_count >= 3
}

/// Return `true` when the user appears to have pending (unsent) input in the
/// %0 prompt — i.e. they are mid-typing and have not yet pressed Enter.
///
/// Uses pane *content* rather than cursor_x so that multiline messages are
/// detected correctly.  When the user presses Enter mid-message the cursor
/// moves to column 0 on a new line, so a cursor_x check would incorrectly
/// report no pending input.
///
/// Strategy:
/// 1. Get the cursor position (cursor_x, cursor_y) from tmux.
/// 2. Capture the exact line the cursor is on and check it for user text.
/// 3. Also capture the last 5 lines of the pane and check ALL of them —
///    this catches multiline input where the cursor line itself is blank but
///    earlier lines have content.
///
/// "User text" means non-whitespace content that remains after stripping ANSI
/// escape sequences and leading prompt characters (>, ❯, $, #, %, │, |).
///
/// Returns `false` on any error (safe default — if we can't tell, assume no
/// pending input rather than permanently suppressing the heartbeat).
fn orchestrator_has_pending_input() -> bool {
    // ── helpers ──────────────────────────────────────────────────────────────

    /// Strip ANSI escape sequences from a string.
    fn strip_ansi(s: &str) -> String {
        // Matches CSI sequences like ESC[…m, ESC[…G, etc.
        let mut out = String::with_capacity(s.len());
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                // Skip ESC [
                i += 2;
                // Skip parameter and intermediate bytes (0x20–0x3F range) and
                // final byte (0x40–0x7E).
                while i < bytes.len() && bytes[i] >= 0x20 && bytes[i] <= 0x3f {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] >= 0x40 && bytes[i] <= 0x7e {
                    i += 1;
                }
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
        out
    }

    /// Return `true` if the line has user-typed content beyond prompt chars.
    fn line_has_content(raw: &str) -> bool {
        let stripped = strip_ansi(raw);
        // Strip leading prompt-style characters and whitespace.
        let trimmed = stripped.trim_start_matches(|c: char| {
            matches!(c, '>' | '$' | '#' | '%' | '│' | '|') || c.is_whitespace()
        });
        // Also strip the Unicode heavy right-angle quotation mark used by
        // some shells (❯, U+276F).
        let trimmed = trimmed
            .trim_start_matches('❯')
            .trim_start_matches(char::is_whitespace);
        !trimmed.is_empty()
    }

    // ── step 1: get cursor position ──────────────────────────────────────────

    let pos_output = match std::process::Command::new("tmux")
        .args([
            "display-message",
            "-t",
            "%0",
            "-p",
            "#{cursor_x} #{cursor_y} #{pane_height}",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let pos_str = match std::str::from_utf8(&pos_output.stdout) {
        Ok(s) => s.trim().to_string(),
        Err(_) => return false,
    };

    // Parse "cursor_x cursor_y pane_height"
    let parts: Vec<&str> = pos_str.split_whitespace().collect();
    if parts.len() < 2 {
        return false;
    }
    let cursor_y: i64 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };

    // ── step 2: check the exact cursor line ──────────────────────────────────

    let cursor_line_output = std::process::Command::new("tmux")
        .args([
            "capture-pane",
            "-t",
            "%0",
            "-p",
            "-S",
            &cursor_y.to_string(),
            "-E",
            &cursor_y.to_string(),
        ])
        .output();

    if let Ok(o) = cursor_line_output {
        if o.status.success() {
            if let Ok(text) = std::str::from_utf8(&o.stdout) {
                if line_has_content(text) {
                    return true;
                }
            }
        }
    }

    // ── step 3: check the last 5 lines (catches multiline input) ────────────

    let last5_output = std::process::Command::new("tmux")
        .args(["capture-pane", "-t", "%0", "-p", "-S", "-5"])
        .output();

    if let Ok(o) = last5_output {
        if o.status.success() {
            if let Ok(text) = std::str::from_utf8(&o.stdout) {
                for line in text.lines() {
                    if line_has_content(line) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Send an unconditional [HEARTBEAT] status message to %0.
///
/// The message includes:
/// - number of active worker panes
/// - current local time (HH:MM)
/// - whether any workers need attention or if everything is nominal
///
/// Skips sending if %0 appears to be actively processing (spinner chars,
/// build steps, or agent tool calls detected in the last 5 lines of output).
/// Also fires a tmux flash notification so the message is visible in the
/// status bar even when the orchestrator pane is not focused.
///
/// Returns `true` when the heartbeat was sent; `false` when it was skipped.
pub fn heartbeat() -> Result<bool> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Check disabled flag and timed snooze: skip without sending if either is active.
    {
        let state = read_heartbeat_state();
        if state.disabled {
            eprintln!("[watch] heartbeat skipped — toggled off (disabled)");
            return Ok(false);
        }
        if state.snooze_until > now {
            let remaining = state.snooze_until - now;
            eprintln!(
                "[watch] heartbeat skipped — snoozed for {remaining}s more (until unix {})",
                state.snooze_until
            );
            return Ok(false);
        }
    }

    // Skip if %0 looks busy to avoid interrupting an active response.
    if orchestrator_is_busy() {
        eprintln!("[watch] heartbeat skipped — %0 appears to be actively processing");
        return Ok(false);
    }

    // Skip if the user has unsent input in the %0 prompt — we must never
    // clobber what the user is typing.
    if orchestrator_has_pending_input() {
        eprintln!("[watch] heartbeat skipped — %0 has unsent input in prompt");
        return Ok(false);
    }

    let all_panes = tmux::list().unwrap_or_default();
    let worker_panes: Vec<_> = all_panes.iter().filter(|p| p.id != "%0").collect();
    let worker_count = worker_panes.len();
    let time = time_hhmm();

    let status_part = if worker_count == 0 {
        "No workers running".to_string()
    } else {
        // Count workers that are stalled or waiting for approval.
        let monitor_state = load_state();
        let needs_attention = worker_panes
            .iter()
            .filter(|p| {
                matches!(
                    classify_pane(&p.id, &monitor_state, 60),
                    Ok(h)
                        if matches!(
                            h.status,
                            HealthStatus::Stalled | HealthStatus::Waiting
                        )
                )
            })
            .count();

        if needs_attention == 0 {
            "All systems nominal".to_string()
        } else {
            format!("{needs_attention} worker(s) need attention")
        }
    };

    let msg = format!("[HEARTBEAT] Active workers: {worker_count} | Time: {time} | {status_part}");

    tmux::send("%0", &msg)?;
    let _ = tmux::flash_notification(&msg);

    Ok(true)
}

// ---------------------------------------------------------------------------
// Heartbeat state persistence
// ---------------------------------------------------------------------------

/// On-disk record written after every heartbeat attempt.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HeartbeatState {
    /// Unix timestamp of the last heartbeat attempt (sent or skipped).
    pub last_beat_ts: u64,
    /// Configured heartbeat interval in seconds.
    pub interval_secs: u64,
    /// Whether the last beat was actually sent to %0 (`false` = skipped).
    pub last_sent: bool,
    /// Predicted unix timestamp of the next beat attempt.
    pub next_beat_ts: u64,
    /// True when at least one worker is stalled/waiting and needs attention.
    pub needs_attention: bool,
    /// Unix timestamp until which heartbeats are snoozed (0 = not snoozed).
    /// Set by `superharness heartbeat --snooze N`.
    /// Only used for timed snoozes — do NOT use as a toggle proxy.
    #[serde(default)]
    pub snooze_until: u64,
    /// When true, heartbeats are permanently disabled until explicitly toggled
    /// back on.  This field survives restarts and has no expiry math.
    /// Set/cleared by `superharness heartbeat` (the toggle subcommand).
    #[serde(default)]
    pub disabled: bool,
}

/// Return the path to the heartbeat state file.
/// Stored alongside events.json in the project-local .superharness/ directory.
pub fn heartbeat_state_path() -> std::path::PathBuf {
    project::get_project_state_dir()
        .map(|d| d.join("heartbeat_state.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/superharness-heartbeat-state.json"))
}

/// Write heartbeat state with explicit snooze_until and disabled values.
pub fn write_heartbeat_state_full(
    last_beat_ts: u64,
    interval_secs: u64,
    sent: bool,
    needs_attention: bool,
    snooze_until: u64,
    disabled: bool,
) {
    let state = HeartbeatState {
        last_beat_ts,
        interval_secs,
        last_sent: sent,
        next_beat_ts: last_beat_ts + interval_secs,
        needs_attention,
        snooze_until,
        disabled,
    };
    let path = heartbeat_state_path();
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        let _ = std::fs::write(&path, json);
    }
}

/// Read the heartbeat state from disk (returns default if file is missing).
pub fn read_heartbeat_state() -> HeartbeatState {
    let path = heartbeat_state_path();
    if !path.exists() {
        return HeartbeatState::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Status counts — lightweight active/total worker summary for the status bar
// ---------------------------------------------------------------------------

/// Return a `"X/Y"` string for the tmux status bar:
/// - X = workers with recently-changed output (active per monitor state)
/// - Y = total workers (excluding the orchestrator pane %0)
///
/// "Active" is defined as: `stall_count == 0` in the persisted monitor state,
/// OR the pane has never been seen by the monitor (new pane — assumed active).
///
/// This avoids reading pane output so it stays lightweight for the 5-second
/// status-bar refresh cycle.
pub fn status_counts() -> String {
    let all_panes = tmux::list().unwrap_or_default();
    let workers: Vec<_> = all_panes.iter().filter(|p| p.id != "%0").collect();
    let total = workers.len();

    if total == 0 {
        return "0/0".to_string();
    }

    let monitor_state = load_state();
    let active = workers
        .iter()
        .filter(|p| {
            // Active if stall_count is 0 (output changed on last monitor check)
            // or if the pane has never been seen by the monitor (new — assume active).
            monitor_state.stall_counts.get(&p.id).copied().unwrap_or(0) == 0
        })
        .count();

    format!("{active}/{total}")
}
