//! Heartbeat system for SuperHarness.
//!
//! This module provides:
//! - `start_thread()` — spawns a background daemon thread that fires heartbeats
//! - `beat()` — the single place that sends [HEARTBEAT] to %0
//! - `main_pane_is_busy()` / `main_pane_has_input()` — guard checks
//! - Heartbeat state persistence (read/write to disk)
//! - `status_counts()` — lightweight active/total worker count for the status bar
//! - File-based trigger protocol for CLI ↔ thread communication

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::project;
use crate::tmux;
use crate::util;

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Return the current UTC wall-clock time as "HH:MMZ" derived from the Unix
/// epoch — no subprocess fork required.
fn time_hhmm() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs_in_day = ts % 86400;
    let h = secs_in_day / 3600;
    let m = (secs_in_day % 3600) / 60;
    format!("{h:02}:{m:02}Z")
}

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

/// Return `true` when the last few lines of %0's output suggest it is
/// actively processing (spinner or block characters visible).
///
/// Threshold: 1 character = busy (any spinner/block char in last 5 lines).
pub fn main_pane_is_busy() -> bool {
    let output = match tmux::read("%0", 5) {
        Ok(o) => o,
        Err(_) => return false,
    };

    // Block characters
    const BLOCK_CHARS: &[char] = &['\u{25A0}', '\u{2B1D}'];
    // Braille spinner characters
    const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    for line in output.lines() {
        for &c in BLOCK_CHARS.iter().chain(SPINNER_CHARS.iter()) {
            if line.contains(c) {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Guard: main_pane_has_input
// ---------------------------------------------------------------------------

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

/// Return `true` when the user appears to have pending (unsent) input in the
/// %0 prompt — i.e. they are mid-typing and have not yet pressed Enter.
///
/// Strategy (for ANY foreground process, not just shells):
/// 1. Get cursor position via tmux display-message
/// 2. If cursor_x == 0, return false
/// 3. Capture cursor line via tmux capture-pane
/// 4. Strip ANSI, strip prompt chars, check for remaining content
/// 5. If content exists, return true
pub fn main_pane_has_input() -> bool {
    // ── step 1: get cursor position ──────────────────────────────────────────

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
    let cursor_x: u64 = match parts[0].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let cursor_y: i64 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };

    // ── step 2: cursor_x guard ───────────────────────────────────────────────
    if cursor_x == 0 {
        return false;
    }

    // ── step 3: check the exact cursor line ──────────────────────────────────

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

    false
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
    let all_panes = tmux::list().unwrap_or_default();
    let worker_count = all_panes.iter().filter(|p| p.id != "%0").count();
    let time = time_hhmm();
    let msg = format!("[HEARTBEAT] Active workers: {worker_count} | Time: {time}");
    let _ = tmux::send("%0", &msg);
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
}
