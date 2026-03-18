//! Heartbeat system for SuperHarness.
//!
//! This module provides:
//! - `start_thread()` — spawns a background daemon thread that fires heartbeats
//! - `beat()` — the single place that sends [HEARTBEAT] to %0
//! - `main_pane_is_busy()` / `main_pane_has_input()` — guard checks
//! - Heartbeat state persistence (read/write to disk)
//! - `status_counts()` — lightweight active/total worker count for the status bar
//! - File-based trigger protocol for CLI ↔ thread communication
//! - Stale worker detection daemon — scans workers every 5s for permission
//!   prompts and stale/idle states, triggering early heartbeats when needed

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::project;
use crate::tmux;
use crate::util;

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Return the current unix timestamp in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Guard: main_pane_is_busy
// ---------------------------------------------------------------------------

/// Braille spinner characters used by Claude Code and opencode.
const BRAILLE_SPINNERS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Determine whether the harness running in %0 is busy from a single snapshot.
///
/// This is the pure-logic core of [`main_pane_is_busy`] — it takes the raw
/// string returned by `tmux capture-pane` and returns `true` when the harness
/// appears to be actively processing (streaming a response, executing a tool,
/// waiting for an API response, etc.).
///
/// Detection order:
///
/// 1. **opencode** (TUI) — bottom bar shows `esc interrupt` when busy,
///    `tab agents` / `commands` when idle.
/// 2. **Codex CLI** (Rust TUI) — shows `esc to interrupt` and/or `Working`
///    when busy, `? for shortcuts` when idle.
/// 3. **Claude Code** (CLI) — braille spinner characters `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`
///    appear in the bottom lines while processing.
/// 4. No pattern matched → **not busy** (safe default: allow heartbeat).
fn extract_busy_state(raw_pane: &str) -> bool {
    let lines: Vec<&str> = raw_pane.lines().collect();
    let len = lines.len();

    // Collect the bottom 10 lines (stripped of ANSI) for scanning.
    let bottom_10: Vec<String> = lines[len.saturating_sub(10)..]
        .iter()
        .map(|l| strip_ansi(l))
        .collect();

    // Narrow view: bottom 5 lines for footer-specific checks.
    let bottom_5: Vec<&str> = bottom_10[bottom_10.len().saturating_sub(5)..]
        .iter()
        .map(|s| s.as_str())
        .collect();

    // ── 1. opencode detection ────────────────────────────────────────────────
    // opencode always renders a footer line below the ╹▀▀ input-box border.
    // Busy:  "⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt"
    // Idle:  "... ctrl+t variants  tab agents  ctrl+p commands  • OpenCode ..."
    //
    // Check if this looks like an opencode pane first (has ╹▀ border or
    // "OpenCode" in the footer).
    let is_opencode = bottom_10
        .iter()
        .any(|l| l.contains("OpenCode") || (l.contains('╹') && l.contains('▀')));

    if is_opencode {
        // "esc interrupt" in the footer → busy
        let has_esc_interrupt = bottom_5
            .iter()
            .any(|l| l.contains("esc interrupt") || l.contains("esc again to interrupt"));
        // "tab agents" or "commands" in the footer → idle
        let has_idle_hints = bottom_5
            .iter()
            .any(|l| l.contains("tab agents") || l.contains("commands"));

        // "esc interrupt" is the definitive busy signal — it takes priority
        // even when idle hints (tab agents, commands) appear on the same line.
        if has_esc_interrupt {
            return true;
        }
        if has_idle_hints {
            return false;
        }
        // opencode detected but neither signal clear — fall through to
        // generic checks below.
    }

    // ── 2. Codex CLI detection ───────────────────────────────────────────────
    // Busy:  "• Working (Ns • esc to interrupt)"  or  "tab to queue"
    // Idle:  "? for shortcuts   N% context left"
    let has_codex_busy = bottom_10.iter().any(|l| {
        l.contains("esc to interrupt")
            || (l.contains("Working") && l.contains('•'))
            || l.contains("tab to queue")
    });

    if has_codex_busy {
        return true;
    }

    let has_codex_idle = bottom_5
        .iter()
        .any(|l| l.contains("? for shortcuts") || l.contains("% context left"));

    // If we see Codex idle markers and no busy markers → not busy.
    if has_codex_idle {
        return false;
    }

    // ── 3. Claude Code detection (braille spinners) ──────────────────────────
    // Braille spinner chars only appear during processing.  Count them in the
    // bottom 10 lines — even a single one is a strong busy signal.
    let has_braille = bottom_10
        .iter()
        .any(|l| l.chars().any(|c| BRAILLE_SPINNERS.contains(&c)));

    if has_braille {
        return true;
    }

    // ── 4. No pattern matched → not busy ─────────────────────────────────────
    false
}

/// Return `true` when the harness in %0 is busy (processing, streaming,
/// executing tools).
///
/// Uses single-snapshot pattern matching — no sleeping or diffing.
pub fn is_harness_busy() -> bool {
    let snap = match tmux::read("%0", 15) {
        Ok(o) => o,
        Err(_) => return false,
    };
    extract_busy_state(&snap)
}

/// Return `true` when %0's harness is busy.
///
/// Delegates to [`is_harness_busy`] which uses single-snapshot pattern
/// matching instead of the old two-snapshot diff approach.
pub fn main_pane_is_busy() -> bool {
    is_harness_busy()
}

// ---------------------------------------------------------------------------
// Guard: main_pane_has_input
// ---------------------------------------------------------------------------

/// Strip ANSI escape sequences from a string.
///
/// Iterates over UTF-8 chars (not raw bytes) so that multi-byte characters
/// like box-drawing `┃` (U+2503) are preserved intact.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Check for CSI sequence: ESC [
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                              // Skip parameter bytes (0x20..=0x3f)
                while let Some(&p) = chars.peek() {
                    if (' '..='?').contains(&p) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                // Skip final byte (0x40..=0x7e)
                if let Some(&f) = chars.peek() {
                    if ('@'..='~').contains(&f) {
                        chars.next();
                    }
                }
            }
            // else: lone ESC — just skip it
        } else {
            out.push(c);
        }
    }
    out
}

/// Characters considered prompt chrome — prompt indicators, box-drawing,
/// block elements, and common shell prompt symbols.
fn is_prompt_chrome(c: char) -> bool {
    matches!(c, '>' | '$' | '#' | '%' | '│' | '|' | '❯')
        || c.is_whitespace()
        || ('\u{2500}'..='\u{257F}').contains(&c) // Box Drawing
        || ('\u{2580}'..='\u{259F}').contains(&c) // Block Elements
}

/// Return `true` if the line has user-typed content beyond prompt chars.
fn line_has_content(raw: &str) -> bool {
    let stripped = strip_ansi(raw);
    let trimmed = stripped
        .trim_start_matches(is_prompt_chrome)
        .trim_start_matches(char::is_whitespace);
    !trimmed.is_empty()
}

/// Return `true` if the line consists entirely of prompt chrome (non-empty
/// but no user content).  This means the line is part of an active prompt
/// border/indicator — e.g. a bare `┃` in opencode's input area.
#[cfg(test)]
fn line_is_prompt_chrome(raw: &str) -> bool {
    let stripped = strip_ansi(raw);
    let trimmed = stripped.trim();
    // Must have at least one non-whitespace char to count as chrome
    // (a fully blank line is just empty, not a prompt indicator).
    if trimmed.is_empty() {
        return false;
    }
    trimmed.chars().all(is_prompt_chrome)
}

/// Extract the user's prompt text from raw pane output.
///
/// This is the pure-logic core of [`get_prompt_text`] — it takes the raw
/// string returned by `tmux capture-pane` and returns whatever text the user
/// has typed into the harness's input area.  Returns an empty string when the
/// input area is empty or when the harness layout is not recognised.
///
/// Detection order:
/// 1. **opencode** — TUI with `┃`-bordered input box above a `╹▀▀▀` border.
/// 2. **Codex CLI** — TUI with `▌` (U+258C) left-border input area.
/// 3. Falls through to empty string (caller can try cursor-based fallback).
fn extract_prompt_text(raw_pane: &str) -> String {
    let lines: Vec<&str> = raw_pane.lines().collect();

    // ── 1. opencode detection ────────────────────────────────────────────────
    // Find the bottom border: a line containing ╹ followed by ▀
    // Then collect ┃-prefixed lines above it.
    if let Some(border_idx) = lines.iter().rposition(|l| {
        let s = strip_ansi(l);
        let t = s.trim_start();
        t.starts_with('╹') && t.contains('▀')
    }) {
        let mut input_lines: Vec<&str> = Vec::new();
        // Walk upward from the line just above the border
        for i in (0..border_idx).rev() {
            let s = strip_ansi(lines[i]);
            let t = s.trim_start();
            if t.starts_with('┃') {
                input_lines.push(lines[i]);
            } else {
                break;
            }
        }
        input_lines.reverse();

        // Drop the last ┃ line — it's the status line (model/plan info)
        if input_lines.len() > 1 {
            input_lines.pop();
        } else {
            // Only one ┃ line (the status line itself) — input area is empty
            return String::new();
        }

        // Strip ┃ prefix and extract user text
        let mut text_parts: Vec<String> = Vec::new();
        for raw_line in &input_lines {
            let stripped = strip_ansi(raw_line);
            // Find the ┃ character and take everything after it
            if let Some(pos) = stripped.find('┃') {
                let after = &stripped[pos + '┃'.len_utf8()..];
                text_parts.push(after.to_string());
            }
        }

        let joined = text_parts.join("\n");
        let trimmed = joined.trim();
        return trimmed.to_string();
    }

    // ── 2. Codex CLI detection ───────────────────────────────────────────────
    // Look for lines with ▌ (U+258C LEFT HALF BLOCK) as the input border.
    // Scan from the bottom for the first ▌-prefixed line.
    if let Some(codex_idx) = lines.iter().rposition(|l| {
        let s = strip_ansi(l);
        let t = s.trim_start();
        t.starts_with('▌')
    }) {
        // Collect consecutive ▌-prefixed lines upward
        let mut input_lines: Vec<&str> = Vec::new();
        for i in (0..=codex_idx).rev() {
            let s = strip_ansi(lines[i]);
            let t = s.trim_start();
            if t.starts_with('▌') {
                input_lines.push(lines[i]);
            } else {
                break;
            }
        }
        input_lines.reverse();

        let mut text_parts: Vec<String> = Vec::new();
        for raw_line in &input_lines {
            let stripped = strip_ansi(raw_line);
            if let Some(pos) = stripped.find('▌') {
                let after = &stripped[pos + '▌'.len_utf8()..];
                text_parts.push(after.to_string());
            }
        }

        let joined = text_parts.join("\n");
        let trimmed = joined.trim();
        return trimmed.to_string();
    }

    // ── 3. No TUI pattern recognised — return empty ─────────────────────────
    // The caller (get_prompt_text) will try the cursor-based fallback for
    // plain CLI harnesses like Claude Code.
    String::new()
}

/// Return the text currently in the %0 prompt input area.
///
/// Works across harnesses:
/// - **opencode** — scans the TUI input box (┃-bordered area)
/// - **Codex CLI** — scans the ▌-bordered input area
/// - **Claude Code** — falls back to cursor-position-based detection
///
/// Returns an empty string when the input area is empty or undetectable.
pub fn get_prompt_text() -> String {
    // Try TUI-based detection first (opencode / Codex)
    let pane_text = match tmux::read("%0", 25) {
        Ok(t) => t,
        Err(_) => return String::new(),
    };

    let tui_result = extract_prompt_text(&pane_text);
    if !tui_result.is_empty() {
        return tui_result;
    }

    // If no TUI pattern found AND the pane *does* have ┃ or ▌ lines, the
    // input box was found but is empty — don't fall through to cursor-based.
    {
        let has_tui_chrome = pane_text.lines().any(|l| {
            let s = strip_ansi(l);
            let t = s.trim_start();
            t.starts_with('┃') || t.starts_with('▌')
        });
        if has_tui_chrome {
            return String::new();
        }
    }

    // ── Fallback: cursor-based detection (Claude Code / plain CLI) ───────────
    let pos_output = match std::process::Command::new("tmux")
        .args([
            "display-message",
            "-t",
            "%0",
            "-p",
            "#{cursor_x} #{cursor_y}",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return String::new(),
    };

    let pos_str = match std::str::from_utf8(&pos_output.stdout) {
        Ok(s) => s.trim().to_string(),
        Err(_) => return String::new(),
    };

    let parts: Vec<&str> = pos_str.split_whitespace().collect();
    if parts.len() < 2 {
        return String::new();
    }
    let cursor_x: u64 = match parts[0].parse() {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    let cursor_y: i64 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    // cursor_x == 0: look back up to 10 lines for content
    if cursor_x == 0 {
        let look_back: i64 = 10;
        let start = (cursor_y - look_back).max(0);
        if start < cursor_y {
            let above_output = std::process::Command::new("tmux")
                .args([
                    "capture-pane",
                    "-t",
                    "%0",
                    "-p",
                    "-S",
                    &start.to_string(),
                    "-E",
                    &(cursor_y - 1).to_string(),
                ])
                .output();

            if let Ok(o) = above_output {
                if o.status.success() {
                    if let Ok(text) = std::str::from_utf8(&o.stdout) {
                        let mut parts: Vec<&str> = Vec::new();
                        for line in text.lines() {
                            if line_has_content(line) {
                                parts.push(line);
                            }
                        }
                        if !parts.is_empty() {
                            return parts.join("\n");
                        }
                    }
                }
            }
        }
        return String::new();
    }

    // cursor_x > 0: capture the cursor line
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
                let stripped = strip_ansi(text);
                let trimmed = stripped
                    .trim_start_matches(is_prompt_chrome)
                    .trim_start_matches(char::is_whitespace)
                    .trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }

    String::new()
}

/// Return `true` when the user appears to have pending (unsent) input in the
/// %0 prompt — i.e. they are mid-typing and have not yet pressed Enter.
pub fn main_pane_has_input() -> bool {
    !get_prompt_text().trim().is_empty()
}

// ---------------------------------------------------------------------------
// Stale worker detection — pure-logic helpers
// ---------------------------------------------------------------------------

/// Scan interval for the worker scanner (in seconds).
/// The scanner runs on a separate counter inside the heartbeat thread loop.
const SCANNER_INTERVAL_SECS: u64 = 5;

/// Number of consecutive unchanged scans before a worker is considered stale.
/// With a 5-second scan interval, 3 scans = 15 seconds of no output change.
const STALE_SCAN_THRESHOLD: u32 = 3;

/// Return `true` if the pane output contains a permission / approval prompt.
///
/// This is intentionally similar to `health::is_waiting_for_permission()` but
/// kept as a standalone pure function in this module to avoid coupling to the
/// health system and to allow independent testing.  The patterns are tuned for
/// the bottom portion of a worker pane (last ~20 lines).
fn has_permission_prompt(output: &str) -> bool {
    // Check the last few non-empty lines — permission prompts sit at the end.
    let tail: String = output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    let lower = tail.to_lowercase();

    // y/n style prompts
    if lower.contains("[y/n]")
        || lower.contains("(y/n)")
        || lower.contains("y/n?")
        || lower.contains("[yes/no]")
        || lower.contains("(yes/no)")
        || lower.contains("yes/no?")
        || lower.contains("(y/n):")
        || lower.contains("yes or no")
    {
        return true;
    }

    // Mixed-case y/N or Y/n variants (opencode shows "Allow bash: ... (Y/n)")
    if lower.contains("(y/n") || lower.contains("[y/n") {
        return true;
    }

    // Generic approval keywords
    if lower.contains("approve?")
        || lower.contains("allow?")
        || lower.contains("allow this?")
        || lower.contains("confirm?")
        || lower.contains("proceed?")
        || lower.contains("continue? [")
        || lower.contains("would you like to")
        || lower.contains("do you want to")
    {
        return true;
    }

    false
}

/// Return `true` if the pane output looks like the worker is asking a question
/// to the orchestrator (e.g. "?" prompts, questions ending with "?").
///
/// This detects cases where a worker is waiting for user input that is NOT a
/// simple y/n permission prompt but rather a freeform question.
fn has_question_prompt(output: &str) -> bool {
    let tail: Vec<&str> = output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .collect();

    let lower_tail: Vec<String> = tail.iter().map(|l| l.to_lowercase()).collect();

    // Claude Code question tool renders "# Questions" headers
    for line in &lower_tail {
        if line.contains("# questions") {
            return true;
        }
    }

    false
}

/// Return `true` if the orchestrator pane (%0) is currently displaying an MCP
/// question dialog.  When this is the case, sending a `[HEARTBEAT]` would
/// interrupt the dialog — so the scanner should suppress early beats.
///
/// Detection: the MCP question tool renders a block starting with `# Questions`
/// followed by option lines containing `(no answer)` or radio-button markers.
fn is_orchestrator_in_question_dialog(orch_output: &str) -> bool {
    let lower = orch_output.to_lowercase();

    // Look for the characteristic MCP question dialog pattern
    if lower.contains("# questions") || lower.contains("(no answer)") {
        return true;
    }

    // Also suppress if the orchestrator is showing a selection menu
    // (radio buttons like "○" or "●" near question text)
    let lines: Vec<&str> = orch_output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(15)
        .collect();

    let has_question_header = lines.iter().any(|l| l.to_lowercase().contains("question"));
    let has_radio_buttons = lines.iter().any(|l| l.contains('○') || l.contains('●'));

    has_question_header && has_radio_buttons
}

/// Determine whether a worker pane needs attention based on its output.
///
/// Returns `true` if:
/// - A permission prompt is detected
/// - A question prompt is detected
///
/// This is a pure function for testability — it does NOT check stale state
/// (that requires the scan-history HashMap maintained by the thread).
fn worker_needs_attention(output: &str) -> bool {
    has_permission_prompt(output) || has_question_prompt(output)
}

// ---------------------------------------------------------------------------
// beat() — the ONLY place that sends to %0
// ---------------------------------------------------------------------------

/// Send a [HEARTBEAT] status message to %0.
///
/// Guards inline: checks busy and input before sending.
/// This is the ONLY function that sends heartbeat messages to %0.
fn beat() {
    if main_pane_is_busy() {
        return;
    }
    if main_pane_has_input() {
        return;
    }
    let _ = tmux::send("%0", "[HEARTBEAT]");
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
// Heartbeat state persistence
// ---------------------------------------------------------------------------

/// On-disk record updated after every heartbeat event.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HeartbeatState {
    /// When true, heartbeats are permanently disabled until explicitly toggled
    /// back on.  Survives restarts.  Set/cleared by `superharness heartbeat-toggle`.
    #[serde(default)]
    pub disabled: bool,
    /// Configured heartbeat interval in seconds.
    #[serde(default)]
    pub interval_secs: u64,
    /// Predicted unix timestamp of the next beat.
    #[serde(default)]
    pub next_beat_ts: u64,
    /// Unix timestamp of the last fired heartbeat (0 = never fired).
    #[serde(default)]
    pub last_beat_ts: u64,
}

/// Return the path to the heartbeat state file.
/// Stored in the project-local `.superharness/` directory.
pub fn heartbeat_state_path() -> std::path::PathBuf {
    project::get_project_state_dir()
        .map(|d| d.join("heartbeat_state.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/superharness-heartbeat-state.json"))
}

/// Write the heartbeat state struct to disk atomically.
///
/// Writes to a `.tmp` sibling first, then renames — on Unix, `rename(2)` is
/// atomic, so concurrent readers always see a complete file.
///
/// Errors are logged to stderr rather than silently dropped.
pub fn write_heartbeat_state(state: &HeartbeatState) {
    let path = heartbeat_state_path();
    let tmp_path = path.with_extension("json.tmp");

    let json = match serde_json::to_string_pretty(state) {
        Ok(j) => j,
        Err(_e) => {
            return;
        }
    };

    if let Err(_e) = std::fs::write(&tmp_path, &json) {
        return;
    }

    if let Err(_e) = std::fs::rename(&tmp_path, &path) {
        // Best-effort cleanup of the orphaned tmp file.
        let _ = std::fs::remove_file(&tmp_path);
    }
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
// File-based trigger paths
// ---------------------------------------------------------------------------

/// Return the path to the heartbeat trigger file.
fn trigger_path() -> std::path::PathBuf {
    project::get_project_state_dir()
        .map(|d| d.join("heartbeat_trigger"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/superharness-heartbeat-trigger"))
}

/// Return the path to the heartbeat snooze file.
fn snooze_path() -> std::path::PathBuf {
    project::get_project_state_dir()
        .map(|d| d.join("heartbeat_snooze"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/superharness-heartbeat-snooze"))
}

/// Return the path to the heartbeat toggle trigger file.
fn toggle_trigger_path() -> std::path::PathBuf {
    project::get_project_state_dir()
        .map(|d| d.join("heartbeat_toggle_trigger"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/superharness-heartbeat-toggle-trigger"))
}

// ---------------------------------------------------------------------------
// Background heartbeat thread
// ---------------------------------------------------------------------------

/// Spawn a background daemon thread that manages heartbeat timing.
///
/// The thread:
/// - Holds state in memory: countdown, interval, disabled, last_beat_ts
/// - Every 1 second:
///   a. If disabled, skip to writing state
///   b. Decrement countdown
///   c. If countdown hits 0, call beat(), reset countdown
///   d. Check trigger file — if exists, delete, beat(), reset countdown
///   e. Check snooze file — if exists, read N, add to countdown, delete
///   f. Check toggle file — if exists, flip disabled, delete
///   g. Write state to disk for heartbeat-status CLI
///
/// The thread is a daemon thread — it dies when main exits.
pub fn start_thread() {
    std::thread::spawn(|| {
        let interval = get_interval();
        let mut countdown: u64 = interval;
        let mut last_beat_ts: u64 = 0;

        // Read any previously persisted state to restore disabled flag
        let prev = read_heartbeat_state();
        let mut disabled: bool = prev.disabled;

        // --- Stale worker scanner state ---
        // Separate countdown so the scanner runs on its own 5-second cycle.
        let mut scanner_countdown: u64 = SCANNER_INTERVAL_SECS;
        // Track output hashes per pane for stale detection.
        // Key: pane_id, Value: (hash of last output, consecutive-unchanged count)
        let mut pane_hashes: HashMap<String, (u64, u32)> = HashMap::new();

        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));

            let now = now_secs();

            // Check for toggle file first (applies whether disabled or not)
            let toggle_path = toggle_trigger_path();
            if toggle_path.exists() {
                let _ = std::fs::remove_file(&toggle_path);
                disabled = !disabled;
                if !disabled {
                    // Re-enabling: reset countdown
                    countdown = interval;
                }
            }

            if !disabled {
                // Decrement countdown
                countdown = countdown.saturating_sub(1);

                // Check if countdown hit 0 — time to beat
                if countdown == 0 {
                    beat();
                    last_beat_ts = now;
                    countdown = interval;
                }

                // Check for trigger file (immediate beat request)
                let trig = trigger_path();
                if trig.exists() {
                    let _ = std::fs::remove_file(&trig);
                    beat();
                    last_beat_ts = now;
                    countdown = interval;
                }

                // Check for snooze file
                let snz = snooze_path();
                if snz.exists() {
                    if let Ok(content) = std::fs::read_to_string(&snz) {
                        if let Ok(n) = content.trim().parse::<u64>() {
                            countdown = countdown.saturating_add(n);
                        }
                    }
                    let _ = std::fs::remove_file(&snz);
                }

                // ── Stale worker scanner ─────────────────────────────────
                // Runs on its own 5-second cycle, independent of the main
                // heartbeat countdown.  When a worker needs attention, it
                // resets the main countdown to 0 so beat() fires immediately
                // (subject to the orchestrator question-dialog suppression).
                scanner_countdown = scanner_countdown.saturating_sub(1);
                if scanner_countdown == 0 {
                    scanner_countdown = SCANNER_INTERVAL_SECS;

                    if let Ok(panes) = tmux::list() {
                        let mut any_needs_attention = false;

                        // Collect live pane IDs so we can prune stale entries later
                        let mut live_ids: Vec<String> = Vec::new();

                        for pane in &panes {
                            // Skip the orchestrator pane
                            if pane.id == "%0" {
                                continue;
                            }
                            live_ids.push(pane.id.clone());

                            // Read last 20 lines of worker output
                            let output = match tmux::read(&pane.id, 20) {
                                Ok(o) => o,
                                Err(_) => continue,
                            };

                            // --- Permission / question prompt detection ---
                            if worker_needs_attention(&output) {
                                any_needs_attention = true;
                                // Reset stale counter since the worker is
                                // actively waiting (not truly "stale")
                                pane_hashes
                                    .insert(pane.id.clone(), (util::hash_string(&output), 0));
                                continue;
                            }

                            // --- Stale/idle detection via output hashing ---
                            let current_hash = util::hash_string(&output);
                            let entry = pane_hashes.entry(pane.id.clone()).or_insert((0, 0));

                            if current_hash == entry.0 {
                                // Output unchanged — increment stale counter
                                entry.1 = entry.1.saturating_add(1);

                                if entry.1 >= STALE_SCAN_THRESHOLD {
                                    // Worker has been unchanged for
                                    // STALE_SCAN_THRESHOLD × SCANNER_INTERVAL_SECS.
                                    // This could mean it finished, crashed,
                                    // or is genuinely idle.
                                    any_needs_attention = true;
                                }
                            } else {
                                // Output changed — reset counter
                                *entry = (current_hash, 0);
                            }
                        }

                        // Prune entries for panes that no longer exist
                        pane_hashes.retain(|id, _| live_ids.contains(id));

                        // If any worker needs attention, trigger an early beat
                        // UNLESS the orchestrator is in a question dialog.
                        if any_needs_attention {
                            let suppress = match tmux::read("%0", 25) {
                                Ok(orch_out) => is_orchestrator_in_question_dialog(&orch_out),
                                Err(_) => false,
                            };

                            if !suppress {
                                // Reset countdown to 0 so beat() fires on
                                // the next tick.
                                countdown = 0;
                            }
                        }
                    }
                }
            }

            // Write state to disk so heartbeat-status CLI can read it
            let next_beat_ts = now + countdown;
            let state = HeartbeatState {
                disabled,
                interval_secs: interval,
                next_beat_ts,
                last_beat_ts,
            };
            write_heartbeat_state(&state);
        }
    });
}

// ---------------------------------------------------------------------------
// Status counts — lightweight active/total worker summary for the status bar
// ---------------------------------------------------------------------------

/// Return the total worker count as a plain number string for the tmux status bar
/// (e.g. "3"). Filters out the orchestrator pane %0.
pub fn status_counts() -> String {
    let all_panes = tmux::list().unwrap_or_default();
    let total = all_panes.iter().filter(|p| p.id != "%0").count();

    total.to_string()
}

// ---------------------------------------------------------------------------
// Pure status-kaomoji helper — extracted for testability
// ---------------------------------------------------------------------------

/// Compute the tmux-formatted kaomoji status string for the given state and
/// current unix timestamp.
///
/// Simplified faces:
/// - Disabled: `(x_x)`
/// - No scheduled beat (`next_beat_ts == 0`): `(^_^) --`
/// - Normal countdown: `(^_^) Ns`
/// - Just fired (within 3s): `(^o^) Ns`
#[cfg(test)]
pub fn status_kaomoji(state: &HeartbeatState, now: u64) -> String {
    // Permanently disabled.
    if state.disabled {
        return "#[fg=colour240](x_x)#[default]".to_string();
    }

    // No scheduled beat.
    if state.next_beat_ts == 0 {
        return "#[fg=colour245](^_^) --#[default]".to_string();
    }

    let secs_since_beat = now.saturating_sub(state.last_beat_ts);
    let secs_to_next = state.next_beat_ts.saturating_sub(now);

    if state.last_beat_ts > 0 && secs_since_beat <= 3 {
        // Just fired — excited, bright green.
        format!("#[fg=colour156](^o^) {secs_to_next}s#[default]")
    } else {
        // Normal — happy, calm green.
        format!("#[fg=colour114](^_^) {secs_to_next}s#[default]")
    }
}

// ---------------------------------------------------------------------------
// Test-only file I/O helpers that accept an explicit path
// ---------------------------------------------------------------------------

/// Read a `HeartbeatState` from an arbitrary path.
#[cfg(test)]
pub(crate) fn read_heartbeat_state_from(path: &std::path::Path) -> HeartbeatState {
    if !path.exists() {
        return HeartbeatState::default();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write a `HeartbeatState` to an arbitrary path.
#[cfg(test)]
pub(crate) fn write_heartbeat_state_to(state: &HeartbeatState, path: &std::path::Path) {
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, json);
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::NamedTempFile;

    // -----------------------------------------------------------------------
    // 1. HeartbeatState defaults
    // -----------------------------------------------------------------------

    #[test]
    fn heartbeat_state_default_values() {
        let s = HeartbeatState::default();
        assert_eq!(s.last_beat_ts, 0, "last_beat_ts must start at 0");
        assert_eq!(s.interval_secs, 0, "interval_secs must start at 0");
        assert_eq!(s.next_beat_ts, 0, "next_beat_ts must start at 0");
        assert!(!s.disabled, "disabled must start false");
    }

    // -----------------------------------------------------------------------
    // 2. Serialization / deserialization round-trips
    // -----------------------------------------------------------------------

    #[test]
    fn serde_round_trip_all_fields() {
        let orig = HeartbeatState {
            last_beat_ts: 1_700_000_000,
            interval_secs: 30,
            next_beat_ts: 1_700_000_030,
            disabled: false,
        };

        let json = serde_json::to_string_pretty(&orig).expect("serialize");
        let restored: HeartbeatState = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.last_beat_ts, orig.last_beat_ts);
        assert_eq!(restored.interval_secs, orig.interval_secs);
        assert_eq!(restored.next_beat_ts, orig.next_beat_ts);
        assert_eq!(restored.disabled, orig.disabled);
    }

    #[test]
    fn serde_round_trip_disabled_true() {
        let orig = HeartbeatState {
            disabled: true,
            last_beat_ts: 42,
            interval_secs: 60,
            next_beat_ts: 102,
        };

        let json = serde_json::to_string_pretty(&orig).unwrap();
        let restored: HeartbeatState = serde_json::from_str(&json).unwrap();

        assert!(restored.disabled);
        assert_eq!(restored.last_beat_ts, 42);
    }

    #[test]
    fn serde_missing_disabled_field_defaults_to_false() {
        let json = r#"{
            "last_beat_ts": 100,
            "interval_secs": 30,
            "next_beat_ts": 130
        }"#;

        let state: HeartbeatState = serde_json::from_str(json).expect("should deserialize");
        assert!(
            !state.disabled,
            "`disabled` must default to false via #[serde(default)]"
        );
    }

    #[test]
    fn serde_ignores_removed_fields_gracefully() {
        let json = r#"{
            "last_beat_ts": 100,
            "interval_secs": 30,
            "last_sent": true,
            "next_beat_ts": 130,
            "needs_attention": false,
            "disabled": false
        }"#;

        let state: HeartbeatState = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(state.last_beat_ts, 100);
        assert_eq!(state.next_beat_ts, 130);
    }

    // -----------------------------------------------------------------------
    // 3. Read/write round-trips with temp files
    // -----------------------------------------------------------------------

    #[test]
    fn read_write_round_trip_temp_file() {
        let tmp = NamedTempFile::new().unwrap();

        let state = HeartbeatState {
            last_beat_ts: 1_700_000_000,
            interval_secs: 60,
            next_beat_ts: 1_700_000_060,
            disabled: false,
        };

        write_heartbeat_state_to(&state, tmp.path());
        let restored = read_heartbeat_state_from(tmp.path());

        assert_eq!(restored.last_beat_ts, state.last_beat_ts);
        assert_eq!(restored.interval_secs, state.interval_secs);
        assert_eq!(restored.next_beat_ts, state.next_beat_ts);
        assert_eq!(restored.disabled, state.disabled);
    }

    #[test]
    fn read_write_preserves_disabled_flag() {
        let tmp = NamedTempFile::new().unwrap();

        let state = HeartbeatState {
            disabled: true,
            ..Default::default()
        };
        write_heartbeat_state_to(&state, tmp.path());

        let restored = read_heartbeat_state_from(tmp.path());
        assert!(restored.disabled, "disabled flag must survive round-trip");
    }

    // -----------------------------------------------------------------------
    // 4. Corrupt / malformed file handling
    // -----------------------------------------------------------------------

    #[test]
    fn read_from_corrupt_file_returns_default() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "{{not valid json!!!").unwrap();
        tmp.flush().unwrap();

        let result = read_heartbeat_state_from(tmp.path());
        assert_eq!(
            result.last_beat_ts, 0,
            "corrupt file → default last_beat_ts"
        );
        assert!(!result.disabled, "corrupt file → default disabled");
    }

    #[test]
    fn read_from_empty_file_returns_default() {
        let tmp = NamedTempFile::new().unwrap();
        let result = read_heartbeat_state_from(tmp.path());
        assert_eq!(result.last_beat_ts, 0);
        assert_eq!(result.interval_secs, 0);
    }

    #[test]
    fn read_from_missing_file_returns_default() {
        let path =
            std::path::PathBuf::from("/tmp/superharness-test-NONEXISTENT-file-abc12345.json");
        let _ = std::fs::remove_file(&path);

        let result = read_heartbeat_state_from(&path);
        assert_eq!(result.last_beat_ts, 0);
        assert!(!result.disabled);
    }

    #[test]
    fn read_from_wrong_type_json_returns_default() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "[1, 2, 3]").unwrap();
        tmp.flush().unwrap();

        let result = read_heartbeat_state_from(tmp.path());
        assert_eq!(result.last_beat_ts, 0);
    }

    #[test]
    fn read_from_null_json_returns_default() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "null").unwrap();
        tmp.flush().unwrap();

        let result = read_heartbeat_state_from(tmp.path());
        assert_eq!(result.last_beat_ts, 0);
        assert!(!result.disabled);
    }

    // -----------------------------------------------------------------------
    // 5. Toggle transitions
    // -----------------------------------------------------------------------

    #[test]
    fn toggle_off_sets_disabled_flag() {
        let mut state = HeartbeatState::default();
        assert!(!state.disabled, "should start enabled");
        state.disabled = true;
        assert!(state.disabled);
    }

    #[test]
    fn toggle_on_clears_disabled_flag() {
        let mut state = HeartbeatState {
            disabled: true,
            ..Default::default()
        };
        state.disabled = false;
        assert!(!state.disabled);
    }

    #[test]
    fn toggle_on_resets_countdown() {
        let now = 1_700_000_000u64;
        let interval = 30u64;
        let mut state = HeartbeatState {
            disabled: true,
            interval_secs: interval,
            next_beat_ts: 0,
            ..Default::default()
        };

        state.disabled = false;
        state.next_beat_ts = now + state.interval_secs;

        assert!(!state.disabled, "must be enabled after toggle");
        assert_eq!(
            state.next_beat_ts,
            now + interval,
            "countdown must restart at now + interval"
        );
    }

    #[test]
    fn toggle_survives_file_round_trip() {
        let tmp = NamedTempFile::new().unwrap();

        let state = HeartbeatState {
            disabled: true,
            last_beat_ts: 1_000,
            interval_secs: 30,
            next_beat_ts: 1_030,
        };
        write_heartbeat_state_to(&state, tmp.path());

        let mut loaded = read_heartbeat_state_from(tmp.path());
        assert!(loaded.disabled);
        loaded.disabled = false;
        loaded.next_beat_ts = 2_000 + loaded.interval_secs;
        write_heartbeat_state_to(&loaded, tmp.path());

        let final_state = read_heartbeat_state_from(tmp.path());
        assert!(!final_state.disabled, "disabled=false must be persisted");
        assert_eq!(final_state.next_beat_ts, 2_030);
    }

    // -----------------------------------------------------------------------
    // 6. Interval computation
    // -----------------------------------------------------------------------

    #[test]
    fn interval_next_beat_equals_last_beat_plus_interval() {
        let last = 1_700_000_000u64;
        let interval = 45u64;
        let state = HeartbeatState {
            last_beat_ts: last,
            interval_secs: interval,
            next_beat_ts: last + interval,
            ..Default::default()
        };
        assert_eq!(state.next_beat_ts, state.last_beat_ts + state.interval_secs);
    }

    #[test]
    fn interval_round_trips_via_temp_file() {
        let tmp = NamedTempFile::new().unwrap();

        let last = 1_700_000_000u64;
        let interval = 45u64;
        let state = HeartbeatState {
            last_beat_ts: last,
            interval_secs: interval,
            next_beat_ts: last + interval,
            disabled: false,
        };

        write_heartbeat_state_to(&state, tmp.path());
        let restored = read_heartbeat_state_from(tmp.path());

        assert_eq!(restored.next_beat_ts, last + interval);
        assert_eq!(restored.interval_secs, interval);
    }

    #[test]
    fn interval_zero_does_not_panic() {
        let state = HeartbeatState {
            interval_secs: 0,
            last_beat_ts: 1000,
            next_beat_ts: 1000,
            ..Default::default()
        };
        assert_eq!(state.next_beat_ts, state.last_beat_ts + state.interval_secs);
    }

    // -----------------------------------------------------------------------
    // 7. Status display kaomoji
    // -----------------------------------------------------------------------

    #[test]
    fn kaomoji_disabled_shows_dead_face() {
        let now = 1_700_000_100u64;
        let state = HeartbeatState {
            disabled: true,
            ..Default::default()
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains("(x_x)"),
            "disabled should show dead face, got: {face}"
        );
    }

    #[test]
    fn kaomoji_no_scheduled_beat_shows_placeholder() {
        let now = 1_700_000_100u64;
        let state = HeartbeatState {
            next_beat_ts: 0,
            disabled: false,
            ..Default::default()
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains("(^_^)") && face.contains("--"),
            "no-scheduled-beat placeholder not found in: {face}"
        );
    }

    #[test]
    fn kaomoji_shows_countdown_when_last_beat_ts_zero_but_next_beat_ts_set() {
        let now = 1_700_000_100u64;
        let state = HeartbeatState {
            last_beat_ts: 0,
            next_beat_ts: now + 25,
            interval_secs: 30,
            disabled: false,
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains("25s"),
            "should show countdown even when last_beat_ts==0 but next_beat_ts is set, got: {face}"
        );
        assert!(
            face.contains("(^_^)"),
            "should show happy face for normal countdown, got: {face}"
        );
    }

    #[test]
    fn kaomoji_just_fired_shows_excited_face() {
        let now = 1_700_000_100u64;
        let state = HeartbeatState {
            last_beat_ts: now - 2,
            next_beat_ts: now + 28,
            interval_secs: 30,
            disabled: false,
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains("(^o^)"),
            "just-fired beat should show excited face, got: {face}"
        );
    }

    #[test]
    fn kaomoji_normal_shows_happy_face() {
        let now = 1_700_000_100u64;
        let state = HeartbeatState {
            last_beat_ts: now - 60,
            next_beat_ts: now + 30,
            interval_secs: 30,
            disabled: false,
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains("(^_^)"),
            "normal state should show happy face, got: {face}"
        );
    }

    #[test]
    fn kaomoji_contains_countdown_seconds() {
        let now = 1_700_000_100u64;
        let secs_to_next = 17u64;
        let state = HeartbeatState {
            last_beat_ts: now - 60,
            next_beat_ts: now + secs_to_next,
            interval_secs: 30,
            disabled: false,
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains(&format!("{secs_to_next}s")),
            "kaomoji must include countdown '17s', got: {face}"
        );
    }

    #[test]
    fn kaomoji_just_fired_boundary_exactly_3s() {
        let now = 1_700_000_100u64;
        let state = HeartbeatState {
            last_beat_ts: now - 3,
            next_beat_ts: now + 27,
            interval_secs: 30,
            disabled: false,
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains("(^o^)"),
            "beat at exactly 3s boundary should still be excited, got: {face}"
        );
    }

    #[test]
    fn kaomoji_just_past_boundary_4s() {
        let now = 1_700_000_100u64;
        let state = HeartbeatState {
            last_beat_ts: now - 4,
            next_beat_ts: now + 26,
            interval_secs: 30,
            disabled: false,
        };
        let face = status_kaomoji(&state, now);
        assert!(
            face.contains("(^_^)"),
            "beat 4s ago should no longer show excited face, got: {face}"
        );
    }

    // -----------------------------------------------------------------------
    // 8. strip_ansi and line_has_content helpers
    // -----------------------------------------------------------------------

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        let input = "\x1b[31mhello\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "hello");
    }

    #[test]
    fn line_has_content_with_prompt_only() {
        assert!(!line_has_content("> "));
        assert!(!line_has_content("$ "));
        assert!(!line_has_content(""));
        // Note: "❯ " (U+276F) is corrupted by the byte-level strip_ansi(),
        // so the multi-byte prompt char is not cleanly stripped. This is a
        // known limitation — the guard still works in practice because tmux
        // capture-pane typically returns ASCII-representable content.
    }

    #[test]
    fn line_has_content_with_text_after_prompt() {
        assert!(line_has_content("> hello"));
        assert!(line_has_content("$ ls -la"));
        assert!(line_has_content("❯ cargo build"));
    }

    /// When the user is typing a multi-line input and presses Enter, the
    /// cursor moves to a new blank line (cursor_x == 0).  The lines *above*
    /// the cursor still contain user content.  `main_pane_has_input()` now
    /// checks up to 3 lines above cursor_y via `line_has_content()`.
    ///
    /// This test verifies the helper correctly identifies content in lines
    /// that would appear above the cursor in a multi-line prompt scenario.
    #[test]
    fn line_has_content_multiline_above_cursor() {
        // Simulates lines above cursor in a multi-line prompt input:
        // line N-2: "> some long command \"  (has content)
        // line N-1: "  --flag value"        (has content)
        // line N:   ""                      (blank — cursor is here with cursor_x==0)
        //
        // main_pane_has_input() captures lines N-3..N-1 and checks each
        // with line_has_content().  If any returns true, input is detected.
        assert!(line_has_content("> some long command \\"));
        assert!(line_has_content("  --flag value"));
        assert!(!line_has_content("")); // blank cursor line
        assert!(!line_has_content("   ")); // whitespace-only
        assert!(line_has_content("hello")); // bare content (no prompt char)
    }

    /// Verify `line_is_prompt_chrome()` correctly identifies lines that consist
    /// entirely of prompt border/indicator chars (e.g. opencode's `┃`).
    #[test]
    fn line_is_prompt_chrome_detects_tui_borders() {
        // Bare box-drawing chars — these are prompt borders in opencode
        assert!(line_is_prompt_chrome("  ┃"));
        assert!(line_is_prompt_chrome("┃"));
        assert!(line_is_prompt_chrome("  ╹"));
        assert!(line_is_prompt_chrome(" │ "));
        assert!(line_is_prompt_chrome("> "));
        assert!(line_is_prompt_chrome("$ "));
        assert!(line_is_prompt_chrome("❯"));

        // Completely blank or whitespace-only lines are NOT prompt chrome
        assert!(!line_is_prompt_chrome(""));
        assert!(!line_is_prompt_chrome("   "));

        // Lines with actual content are NOT prompt chrome
        assert!(!line_is_prompt_chrome("┃  hello"));
        assert!(!line_is_prompt_chrome("> some input"));
        assert!(!line_is_prompt_chrome("Build  Claude Opus 4.6"));
    }

    // -----------------------------------------------------------------------
    // 9. extract_prompt_text — TUI input area parsing
    // -----------------------------------------------------------------------

    #[test]
    fn extract_prompt_text_opencode_empty() {
        let pane = "\
     Some response text here

     ▣  Build · claude-opus-4-6

  ┃
  ┃
  ┃
  ┃  Plan  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                           • OpenCode 1.2.27";
        assert_eq!(extract_prompt_text(pane), "");
    }

    #[test]
    fn extract_prompt_text_opencode_with_text() {
        let pane = "\
     Some response text here

     ▣  Build · claude-opus-4-6

  ┃ hello world
  ┃
  ┃
  ┃  Plan  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                           • OpenCode 1.2.27";
        assert_eq!(extract_prompt_text(pane), "hello world");
    }

    #[test]
    fn extract_prompt_text_opencode_multiline() {
        let pane = "\
     Some response text here

  ┃ first line
  ┃ second line
  ┃
  ┃  Plan  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                           • OpenCode 1.2.27";
        assert_eq!(extract_prompt_text(pane), "first line\n second line");
    }

    #[test]
    fn extract_prompt_text_opencode_only_status_line() {
        // Only the status ┃ line exists (single line input box = just status)
        let pane = "\
     Some response text here
  ┃  Plan  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                           • OpenCode 1.2.27";
        assert_eq!(extract_prompt_text(pane), "");
    }

    #[test]
    fn extract_prompt_text_codex_empty() {
        let pane = "\
     Some output
▌ 
  ? for shortcuts                              100% context left";
        // Just ▌ + space — should be empty after trimming
        assert_eq!(extract_prompt_text(pane), "");
    }

    #[test]
    fn extract_prompt_text_codex_with_text() {
        let pane = "\
     Some output
▌ hello world
  ? for shortcuts                              100% context left";
        assert_eq!(extract_prompt_text(pane), "hello world");
    }

    #[test]
    fn extract_prompt_text_no_tui_pattern() {
        // Plain CLI output — no ┃ or ▌ patterns
        let pane = "\
> some previous command
output line 1
output line 2
> ";
        // No TUI detected — should return empty (caller uses cursor fallback)
        assert_eq!(extract_prompt_text(pane), "");
    }

    #[test]
    fn extract_prompt_text_opencode_real_capture_empty_input() {
        // Real-world capture: tool output in ┃ blocks ABOVE the input box,
        // separated by non-┃ response text.  Input box is empty.
        let pane = "\
     ▣  Build · claude-opus-4-6 · 2m 11s

  ┃
  ┃  can we somehow test it?
  ┃

     Some response text here.

  ┃
  ┃  $ tmux capture-pane -t %0 -p -S -25
  ┃
  ┃  Click to expand
  ┃

     More response text.

     ▣  Build · claude-opus-4-6

  ┃
  ┃
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                           • OpenCode 1.2.27";
        assert_eq!(extract_prompt_text(pane), "");
    }

    #[test]
    fn extract_prompt_text_opencode_real_capture_with_input() {
        // Same layout but user has typed text
        let pane = "\
     ▣  Build · claude-opus-4-6 · 2m 11s

  ┃
  ┃  previous message
  ┃

     Some response text.

     ▣  Build · claude-opus-4-6

  ┃ fix the heartbeat bug
  ┃
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                           • OpenCode 1.2.27";
        assert_eq!(extract_prompt_text(pane), "fix the heartbeat bug");
    }

    // -----------------------------------------------------------------------
    // 10. extract_busy_state — single-snapshot busy detection
    // -----------------------------------------------------------------------

    #[test]
    fn extract_busy_opencode_idle() {
        // opencode idle: footer shows "tab agents" and "ctrl+p commands",
        // NO "esc interrupt"
        let pane = "\
     Some response text here.

     ▣  Build · claude-opus-4-6

  ┃
  ┃
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
                                    ctrl+t variants  tab agents  ctrl+p commands    • OpenCode 1.2.27";
        assert!(
            !extract_busy_state(pane),
            "opencode idle should NOT be busy"
        );
    }

    #[test]
    fn extract_busy_opencode_busy() {
        // opencode busy: footer shows "esc interrupt", no idle hints
        let pane = "\
     Some response text here.

     ▣  Build · claude-opus-4-6

  ┃
  ┃
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                           • OpenCode 1.2.27";
        assert!(extract_busy_state(pane), "opencode busy should be busy");
    }

    #[test]
    fn extract_busy_opencode_esc_again() {
        // opencode after first Esc press: "esc again to interrupt"
        let pane = "\
     Streaming response...

     ▣  Build · claude-opus-4-6

  ┃
  ┃
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc again to interrupt                   • OpenCode 1.2.27";
        assert!(
            extract_busy_state(pane),
            "opencode 'esc again to interrupt' should be busy"
        );
    }

    #[test]
    fn extract_busy_opencode_real_capture_busy() {
        // Real-world capture from tmux while opencode was processing
        let pane = "\
     → Read src/heartbeat.rs [offset=389, limit=80]
     ✱ Grep \"spinner|braille\" in src (10 matches)

     Now let me look at health.rs:

     → Read src/health.rs [limit=300]

     ~ Writing command...

     ▣  Build · claude-opus-4-6

  ┃
  ┃
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                                                     • OpenCode 1.2.27";
        assert!(
            extract_busy_state(pane),
            "real opencode busy capture should be busy"
        );
    }

    #[test]
    fn extract_busy_codex_idle() {
        // Codex CLI idle: shows "? for shortcuts" and "% context left"
        let pane = "\
     Some previous output here.
     Tool result: success.

▌ 
  ? for shortcuts                              100% context left";
        assert!(!extract_busy_state(pane), "codex idle should NOT be busy");
    }

    #[test]
    fn extract_busy_codex_busy() {
        // Codex CLI busy: shows "Working" with spinner and "esc to interrupt"
        let pane = "\
     Running tool: bash ...

  • Working (5s • esc to interrupt)
    └ Executing bash command

▌ 
  tab to queue message                         72% context left";
        assert!(extract_busy_state(pane), "codex busy should be busy");
    }

    #[test]
    fn extract_busy_codex_busy_esc_only() {
        // Codex CLI busy: only "esc to interrupt" visible
        let pane = "\
     Streaming response text...
     More streaming text...

  • Exploring (12s • esc to interrupt)";
        assert!(
            extract_busy_state(pane),
            "codex 'esc to interrupt' should be busy"
        );
    }

    #[test]
    fn extract_busy_claude_code_idle() {
        // Claude Code idle: prompt with ? or >, no spinners
        let pane = "\
  Some previous output.
  File written successfully.

? ";
        assert!(
            !extract_busy_state(pane),
            "claude code idle should NOT be busy"
        );
    }

    #[test]
    fn extract_busy_claude_code_busy() {
        // Claude Code busy: braille spinners present
        let pane = "\
  Some previous output.

  ⠹ Cogitating…

";
        assert!(
            extract_busy_state(pane),
            "claude code with braille spinner should be busy"
        );
    }

    #[test]
    fn extract_busy_claude_code_busy_multiple_spinners() {
        // Claude Code: multiple spinner chars on one line
        let pane = "\
  Previous response text.
  ⠋ Architecting…
  Running bash command...";
        assert!(
            extract_busy_state(pane),
            "claude code multiple spinners should be busy"
        );
    }

    #[test]
    fn extract_busy_empty_pane() {
        let pane = "";
        assert!(!extract_busy_state(pane), "empty pane should NOT be busy");
    }

    #[test]
    fn extract_busy_blank_lines() {
        let pane = "\n\n\n\n\n";
        assert!(!extract_busy_state(pane), "blank lines should NOT be busy");
    }

    #[test]
    fn extract_busy_generic_text() {
        // No harness-specific patterns — default to not busy
        let pane = "\
user@host:~$ ls -la
total 42
drwxr-xr-x  5 user user 4096 Mar 18 12:00 .
drwxr-xr-x  3 user user 4096 Mar 17 10:00 ..
user@host:~$ ";
        assert!(
            !extract_busy_state(pane),
            "generic shell output should NOT be busy"
        );
    }

    #[test]
    fn extract_busy_opencode_idle_with_context_panel() {
        // opencode idle with the right-side context panel visible
        let pane = "\
     Response complete.                                                              Context
                                                                                     87,449 tokens
     ▣  Build · claude-opus-4-6                                                      9% used

  ┃                                                                                  LSP
  ┃                                                                                  LSPs active
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                  ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
                                    ctrl+t variants  tab agents  ctrl+p commands    • OpenCode 1.2.27";
        assert!(
            !extract_busy_state(pane),
            "opencode idle with context panel should NOT be busy"
        );
    }

    #[test]
    fn extract_busy_opencode_both_indicators_same_line() {
        // Real-world capture: opencode puts BOTH "esc interrupt" AND
        // "tab agents"/"ctrl+p commands" on the same footer line when busy.
        // "esc interrupt" must take priority → busy.
        let pane = "\
     Let me check the current heartbeat and busy detection code.

     ▣  Build · claude-opus-4-6

  ┃
  ┃
  ┃
  ┃  Build  Claude Opus 4.6 Anthropic                                                                        ~/code/superharness:main
  ╹▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
   ⬝⬝⬝⬝⬝⬝⬝⬝  esc interrupt                                                                                   ctrl+t variants  tab agents  ctrl+p commands    • OpenCode 1.2.27";
        assert!(
            extract_busy_state(pane),
            "opencode footer with both 'esc interrupt' and idle hints on same line should be BUSY"
        );
    }

    // -----------------------------------------------------------------------
    // 11. has_permission_prompt — worker permission prompt detection
    // -----------------------------------------------------------------------

    #[test]
    fn permission_prompt_yn_brackets() {
        let output = "\
Running bash command...
Allow this action? [y/n]";
        assert!(
            has_permission_prompt(output),
            "[y/n] should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_yn_parens() {
        let output = "\
Allow bash: rm -rf /tmp/test (y/n)";
        assert!(
            has_permission_prompt(output),
            "(y/n) should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_yn_question() {
        let output = "\
This will delete files. y/n?";
        assert!(
            has_permission_prompt(output),
            "y/n? should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_yes_no_brackets() {
        let output = "\
Proceed with deployment? [yes/no]";
        assert!(
            has_permission_prompt(output),
            "[yes/no] should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_opencode_yn() {
        // opencode shows: "Allow bash: ... (Y/n)"
        let output = "\
Allow bash: cargo build --release (Y/n)";
        assert!(
            has_permission_prompt(output),
            "opencode (Y/n) should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_approve() {
        let output = "\
The tool wants to execute: rm -rf build/
approve?";
        assert!(
            has_permission_prompt(output),
            "approve? should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_allow() {
        let output = "\
Tool request: write to /etc/hosts
allow?";
        assert!(
            has_permission_prompt(output),
            "allow? should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_confirm() {
        let output = "\
About to push to main branch.
confirm?";
        assert!(
            has_permission_prompt(output),
            "confirm? should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_proceed() {
        let output = "\
This will modify 15 files.
proceed?";
        assert!(
            has_permission_prompt(output),
            "proceed? should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_would_you_like() {
        let output = "\
Found 3 type errors.
Would you like to fix them automatically?";
        assert!(
            has_permission_prompt(output),
            "'would you like to' should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_do_you_want() {
        let output = "\
Build failed.
Do you want to retry with --verbose?";
        assert!(
            has_permission_prompt(output),
            "'do you want to' should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_continue_bracket() {
        let output = "\
Installation ready.
continue? [press enter]";
        assert!(
            has_permission_prompt(output),
            "'continue? [' should trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_not_triggered_by_normal_output() {
        let output = "\
Building project...
Compiling 42 crates
Build succeeded in 12.3s
All tests passed.";
        assert!(
            !has_permission_prompt(output),
            "normal build output should NOT trigger permission detection"
        );
    }

    #[test]
    fn permission_prompt_not_triggered_by_question_in_code() {
        // The word "confirm" appears inside code, not as a prompt
        let output = "\
fn confirm_action(action: &str) -> bool {
    println!(\"Confirming: {}\", action);
    true
}";
        assert!(
            !has_permission_prompt(output),
            "code containing 'confirm' should NOT trigger (no '?' suffix)"
        );
    }

    #[test]
    fn permission_prompt_yes_or_no() {
        let output = "\
Delete all temporary files? yes or no";
        assert!(
            has_permission_prompt(output),
            "'yes or no' should trigger permission detection"
        );
    }

    // -----------------------------------------------------------------------
    // 12. has_question_prompt — worker question detection
    // -----------------------------------------------------------------------

    #[test]
    fn question_prompt_mcp_questions_header() {
        let output = "\
# Questions

What database should we use?
○ PostgreSQL
○ SQLite
○ MySQL";
        assert!(
            has_question_prompt(output),
            "'# Questions' header should trigger question detection"
        );
    }

    #[test]
    fn question_prompt_not_normal_output() {
        let output = "\
Building...
Compiling crate...
Done.";
        assert!(
            !has_question_prompt(output),
            "normal output should NOT trigger question detection"
        );
    }

    // -----------------------------------------------------------------------
    // 13. is_orchestrator_in_question_dialog — suppress heartbeats
    // -----------------------------------------------------------------------

    #[test]
    fn orchestrator_question_dialog_detected() {
        let output = "\
Some previous output...

# Questions

Which approach do you prefer?
○ Option A (Recommended)
○ Option B
(no answer)";
        assert!(
            is_orchestrator_in_question_dialog(output),
            "MCP question dialog with '# Questions' and '(no answer)' should be detected"
        );
    }

    #[test]
    fn orchestrator_question_dialog_no_answer_only() {
        let output = "\
Processing...
Selection required (no answer)";
        assert!(
            is_orchestrator_in_question_dialog(output),
            "'(no answer)' alone should trigger suppression"
        );
    }

    #[test]
    fn orchestrator_question_dialog_radio_buttons() {
        let output = "\
Question: What model to use?
● claude-opus-4-6
○ claude-sonnet-4-6
○ claude-haiku-4-5";
        assert!(
            is_orchestrator_in_question_dialog(output),
            "radio buttons with question text should trigger suppression"
        );
    }

    #[test]
    fn orchestrator_no_question_dialog() {
        let output = "\
[HEARTBEAT]
Workers: 3 active
All workers healthy.
> ";
        assert!(
            !is_orchestrator_in_question_dialog(output),
            "normal orchestrator output should NOT trigger suppression"
        );
    }

    #[test]
    fn orchestrator_busy_output_no_dialog() {
        let output = "\
⠹ Processing worker output...
Reading pane %3...
Worker %3 status: working";
        assert!(
            !is_orchestrator_in_question_dialog(output),
            "orchestrator busy output should NOT trigger suppression"
        );
    }

    // -----------------------------------------------------------------------
    // 14. worker_needs_attention — combined detection
    // -----------------------------------------------------------------------

    #[test]
    fn worker_needs_attention_permission_prompt() {
        let output = "\
Running tool...
Allow bash: git push origin main (Y/n)";
        assert!(
            worker_needs_attention(output),
            "permission prompt should trigger needs_attention"
        );
    }

    #[test]
    fn worker_needs_attention_question_prompt() {
        let output = "\
# Questions

Where should the config file be stored?";
        assert!(
            worker_needs_attention(output),
            "question prompt should trigger needs_attention"
        );
    }

    #[test]
    fn worker_needs_attention_normal_output() {
        let output = "\
Compiling superharness v0.1.0
Building [=====     ] 50%
Finished dev profile";
        assert!(
            !worker_needs_attention(output),
            "normal build output should NOT trigger needs_attention"
        );
    }

    #[test]
    fn worker_needs_attention_empty_output() {
        assert!(
            !worker_needs_attention(""),
            "empty output should NOT trigger needs_attention"
        );
    }

    // -----------------------------------------------------------------------
    // 15. Stale detection constants sanity
    // -----------------------------------------------------------------------

    #[test]
    fn stale_constants_reasonable() {
        assert_eq!(
            SCANNER_INTERVAL_SECS, 5,
            "scanner should run every 5 seconds"
        );
        assert_eq!(
            STALE_SCAN_THRESHOLD, 3,
            "stale threshold should be 3 consecutive unchanged scans"
        );
        // Total stale detection time = 5 * 3 = 15 seconds
        assert_eq!(
            SCANNER_INTERVAL_SECS * STALE_SCAN_THRESHOLD as u64,
            15,
            "stale detection should trigger after ~15 seconds"
        );
    }
}
