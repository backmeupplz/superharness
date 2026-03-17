//! Heartbeat and worker status utilities for SuperHarness.
//!
//! This module provides:
//! - `heartbeat()` — sends [HEARTBEAT] messages to the orchestrator pane (%0) unconditionally
//! - `daemon_tick()` — called every 1s by the background daemon; checks countdown and fires
//! - Heartbeat state persistence (read/write to disk)
//! - `status_counts()` — lightweight active/total worker count for the status bar

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::health::{classify_pane, HealthStatus};
use crate::monitor::load_state;
use crate::project;
use crate::tmux;
use crate::util;

// ---------------------------------------------------------------------------
// Daemon pane identity
// ---------------------------------------------------------------------------

/// Window name (and pane title) used for the heartbeat daemon pane.
/// Used to filter the daemon out of worker counts and listings.
pub const DAEMON_WINDOW: &str = "heartbeat-daemon";

/// Return `true` if this pane is the heartbeat daemon.
/// The daemon must be excluded from all worker counts and listings.
pub fn is_daemon_pane(p: &tmux::PaneInfo) -> bool {
    p.window == DAEMON_WINDOW || p.title == DAEMON_WINDOW
}

// ---------------------------------------------------------------------------
// Private helpers
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

    let spinner_count: usize = output
        .lines()
        .map(|line| spinner_chars.iter().filter(|&&c| line.contains(c)).count())
        .sum();

    spinner_count >= 3
}

/// Return `true` when the user appears to have pending (unsent) input in the
/// %0 prompt — i.e. they are mid-typing and have not yet pressed Enter.
///
/// Strategy:
/// 1. Get the cursor position (cursor_x, cursor_y) from tmux.
/// 2. Capture the exact line the cursor is on and check it for user text.
/// 3. Also capture the last 5 lines of the pane and check ALL of them.
///
/// Returns `false` on any error (safe default).
fn orchestrator_has_pending_input() -> bool {
    // ── helpers ──────────────────────────────────────────────────────────────

    /// Strip ANSI escape sequences from a string.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
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
        let trimmed = stripped.trim_start_matches(|c: char| {
            matches!(c, '>' | '$' | '#' | '%' | '│' | '|') || c.is_whitespace()
        });
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

// ---------------------------------------------------------------------------
// Configurable interval
// ---------------------------------------------------------------------------

/// Read `heartbeat_interval` from `~/.config/superharness/config.json`.
/// Returns 30 if the field is missing, the file is unreadable, or parsing fails.
pub fn get_interval() -> u64 {
    let config_path = util::superharness_config_dir().join("config.json");
    if config_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(n) = v["heartbeat_interval"].as_u64() {
                    return n;
                }
            }
        }
    }
    30
}

// ---------------------------------------------------------------------------
// Heartbeat — unconditional send to %0
// ---------------------------------------------------------------------------

/// Send a [HEARTBEAT] status message to %0 unconditionally.
///
/// Self-guards against disabled state and rapid-fire duplicates so callers
/// don't need to check those conditions themselves.
/// Called by:
///   - `daemon_tick()` after all gating checks pass
///   - `handle_heartbeat(None)` when a worker explicitly fires a beat
///   - `handle_kill()` after killing a worker
///
/// Returns `needs_attention` — `true` when at least one worker is stalled or
/// waiting for approval.
pub fn heartbeat() -> Result<bool> {
    let state = read_heartbeat_state();
    // Bug 1 fix: self-guard — honour disabled flag regardless of caller
    if state.disabled {
        return Ok(false);
    }
    // Bug 3 fix: dedup guard — don't pile up messages if fired in quick succession
    let now = util::now_unix();
    if now.saturating_sub(state.last_beat_ts) < 5 {
        return Ok(false);
    }

    let all_panes = tmux::list().unwrap_or_default();
    let worker_panes: Vec<_> = all_panes
        .iter()
        .filter(|p| p.id != "%0" && !is_daemon_pane(p))
        .collect();
    let worker_count = worker_panes.len();
    let time = time_hhmm();

    let (status_part, needs_attention) = if worker_count == 0 {
        ("No workers running".to_string(), false)
    } else {
        let monitor_state = load_state();
        let attention_count = worker_panes
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

        let msg = if attention_count == 0 {
            "All systems nominal".to_string()
        } else {
            format!("{attention_count} worker(s) need attention")
        };
        (msg, attention_count > 0)
    };

    let msg = format!("[HEARTBEAT] Active workers: {worker_count} | Time: {time} | {status_part}");

    tmux::send("%0", &msg)?;
    let _ = tmux::flash_notification(&msg);

    Ok(needs_attention)
}

// ---------------------------------------------------------------------------
// Daemon tick — called every 1s by the heartbeat daemon loop
// ---------------------------------------------------------------------------

/// Process one daemon tick. Called every 1s by the background daemon loop.
///
/// Silent — no stdout output. Logic:
/// 1. Read state. If disabled → return.
/// 2. Stale-state guard: if `next_beat_ts` is >5 min in the past, reset it.
/// 3. If `now < next_beat_ts` → return (countdown still running).
/// 4. If %0 has pending input or is busy → skip beat, reschedule to now+interval.
/// 5. Otherwise → fire heartbeat, update state.
///
/// When a beat is skipped (busy/pending input), `next_beat_ts` is set to
/// `now + interval` — the daemon does NOT retry every second.
pub fn daemon_tick() -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut state = read_heartbeat_state();

    // Ensure interval is populated (defensive: handle fresh/uninitialized state)
    if state.interval_secs == 0 {
        state.interval_secs = get_interval();
    }
    let interval = state.interval_secs;

    // 1. Disabled — do nothing
    if state.disabled {
        return Ok(());
    }

    // 2. Stale-state guard: next_beat_ts more than 5 minutes in the past
    //    Avoids firing a backlog of beats from a previous session.
    if state.next_beat_ts > 0 && now > state.next_beat_ts.saturating_add(300) {
        state.next_beat_ts = now + interval;
        write_heartbeat_state(&state);
        return Ok(());
    }

    // 3. Countdown still running — nothing to do
    if state.next_beat_ts > now {
        return Ok(());
    }

    // 4. %0 has pending input or is busy → skip this beat entirely,
    //    schedule the next one at the normal interval (don't retry every 1s)
    if orchestrator_has_pending_input() || orchestrator_is_busy() {
        state.next_beat_ts = now + interval;
        write_heartbeat_state(&state);
        return Ok(());
    }

    // 5. Re-read state to catch any toggle that happened during this tick
    //    (TOCTOU fix: heartbeat-toggle may have written disabled=true after our
    //    initial read at the top of this function)
    let fresh = read_heartbeat_state();
    if fresh.disabled {
        return Ok(());
    }

    // 5b. All clear — fire the heartbeat
    let needs_attention = heartbeat().unwrap_or(false);

    // Re-read one more time so we only overwrite timing fields and preserve
    // the disabled flag (and any other fields) from the most-recent disk state.
    let mut latest = read_heartbeat_state();
    latest.last_beat_ts = now;
    latest.next_beat_ts = now + interval;
    latest.last_sent = true;
    latest.needs_attention = needs_attention;
    write_heartbeat_state(&latest);

    Ok(())
}

// ---------------------------------------------------------------------------
// Heartbeat state persistence
// ---------------------------------------------------------------------------

/// On-disk record updated after every heartbeat event.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HeartbeatState {
    /// Unix timestamp of the last fired heartbeat (0 = never fired).
    pub last_beat_ts: u64,
    /// Configured heartbeat interval in seconds.
    pub interval_secs: u64,
    /// Whether the last scheduled beat was actually sent to %0 (false = skipped).
    pub last_sent: bool,
    /// Predicted unix timestamp of the next beat.
    pub next_beat_ts: u64,
    /// True when at least one worker is stalled/waiting and needs attention.
    pub needs_attention: bool,
    /// When true, heartbeats are permanently disabled until explicitly toggled
    /// back on.  Survives restarts.  Set/cleared by `superharness heartbeat-toggle`.
    #[serde(default)]
    pub disabled: bool,
}

/// Return the path to the heartbeat state file.
/// Stored in the project-local `.superharness/` directory.
pub fn heartbeat_state_path() -> std::path::PathBuf {
    project::get_project_state_dir()
        .map(|d| d.join("heartbeat_state.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/superharness-heartbeat-state.json"))
}

/// Write the heartbeat state struct to disk as-is.
pub fn write_heartbeat_state(state: &HeartbeatState) {
    let path = heartbeat_state_path();
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(&path, json);
    }
}

/// Convenience writer: `next_beat_ts` is computed as `last_beat_ts + interval_secs`.
/// For fine-grained control over `next_beat_ts` use `write_heartbeat_state()` directly.
#[allow(dead_code)]
pub fn write_heartbeat_state_full(
    last_beat_ts: u64,
    interval_secs: u64,
    sent: bool,
    needs_attention: bool,
    disabled: bool,
) {
    let interval = if interval_secs == 0 {
        get_interval()
    } else {
        interval_secs
    };
    write_heartbeat_state(&HeartbeatState {
        last_beat_ts,
        interval_secs: interval,
        last_sent: sent,
        next_beat_ts: last_beat_ts + interval,
        needs_attention,
        disabled,
    });
}

/// Read the heartbeat state from disk (returns default if file is missing or corrupt).
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

/// Return the total worker count as a plain number string for the tmux status bar
/// (e.g. "3"). Filters out the orchestrator pane %0 and the heartbeat-daemon pane.
pub fn status_counts() -> String {
    let all_panes = tmux::list().unwrap_or_default();
    let total = all_panes
        .iter()
        .filter(|p| p.id != "%0" && !is_daemon_pane(p))
        .count();

    total.to_string()
}
