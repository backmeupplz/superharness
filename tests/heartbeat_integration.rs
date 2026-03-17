//! Integration tests for heartbeat commands.
//!
//! These tests require an active tmux session with a pane %0. Run with:
//!
//!   cargo test -- --ignored
//!
//! Or, to avoid races between tests (they share a global state file):
//!
//!   cargo test -- --ignored --test-threads=1
//!
//! Each test uses an isolated temporary project directory so the real
//! superharness state is never corrupted. A RAII `ProjectGuard` saves and
//! restores `active_project.txt` around every test.
//!
//! A process-wide mutex (`TEST_MUTEX`) ensures tests that modify the shared
//! `active_project.txt` file do not race with each other even when the test
//! runner uses multiple threads.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

// ─────────────────────────────────────────────────────────────────────────────
// Global serialisation lock
// ─────────────────────────────────────────────────────────────────────────────

/// All integration tests that touch `active_project.txt` must hold this lock
/// for their entire duration to prevent races when the test runner spawns
/// multiple threads.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Path of the superharness binary under test.
///
/// `CARGO_BIN_EXE_superharness` is set by Cargo when compiling integration
/// tests — it always points to the binary built from the current source tree.
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_superharness"))
}

/// Current Unix timestamp in whole seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Path to `~/.local/share/superharness/active_project.txt`.
///
/// Mirrors the logic in `src/project.rs::active_project_file()` without
/// depending on the `dirs` crate.
fn active_project_file() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .expect("HOME or USERPROFILE must be set");
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
        .join("active_project.txt")
}

// ─────────────────────────────────────────────────────────────────────────────
// ProjectGuard — isolated project state per test
// ─────────────────────────────────────────────────────────────────────────────

/// RAII helper that:
///   1. Creates a temporary directory for the test's project state.
///   2. Writes that directory to `active_project.txt` so the superharness
///      binary uses an isolated state for this test.
///   3. Creates the `.superharness/` sub-directory inside the temp dir.
///   4. Restores the original `active_project.txt` content on `drop`.
struct ProjectGuard {
    /// Original content of `active_project.txt` (None if the file didn't exist).
    original: Option<String>,
    /// Temporary directory holding `.superharness/heartbeat_state.json`.
    temp_dir: tempfile::TempDir,
}

impl ProjectGuard {
    fn new() -> Self {
        let file = active_project_file();
        let original = std::fs::read_to_string(&file).ok();

        let temp_dir = tempfile::TempDir::new().expect("create temp dir");

        // Ensure the parent dir for active_project.txt exists.
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("create superharness data dir");
        }
        // Point the binary at our temp dir.
        std::fs::write(&file, temp_dir.path().to_string_lossy().as_bytes())
            .expect("write active_project.txt");

        // Pre-create the state directory the binary expects.
        std::fs::create_dir_all(temp_dir.path().join(".superharness"))
            .expect("create .superharness dir");

        Self { original, temp_dir }
    }

    /// Path to the heartbeat state file inside the temp project.
    fn state_file(&self) -> PathBuf {
        self.temp_dir
            .path()
            .join(".superharness")
            .join("heartbeat_state.json")
    }

    /// Write a JSON value as the heartbeat state.
    fn write_state(&self, state: &serde_json::Value) {
        let json = serde_json::to_string_pretty(state).expect("serialize state");
        std::fs::write(self.state_file(), json).expect("write heartbeat_state.json");
    }

    /// Read the heartbeat state back from disk as a JSON value.
    ///
    /// Returns `{}` if the file is missing or unparseable.
    fn read_state(&self) -> serde_json::Value {
        let content =
            std::fs::read_to_string(self.state_file()).unwrap_or_else(|_| "{}".to_string());
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    }
}

impl Drop for ProjectGuard {
    fn drop(&mut self) {
        let file = active_project_file();
        match &self.original {
            Some(orig) => {
                // Best-effort restore — ignore errors in drop.
                let _ = std::fs::write(&file, orig.as_bytes());
            }
            None => {
                // File didn't exist before the test; remove it.
                let _ = std::fs::remove_file(&file);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Group 1 — Fire heartbeat and verify delivery to pane %0
// ─────────────────────────────────────────────────────────────────────────────

/// Run `superharness heartbeat` and verify the [HEARTBEAT] marker arrives in
/// pane %0's visible buffer (captured via `tmux capture-pane`).
///
/// Pre-conditions:
///   * A tmux session named "superharness" is running with a pane %0.
///   * The heartbeat is enabled and the dedup window (5 s) is not active.
///
/// The test writes a state with `last_beat_ts` 60 seconds in the past so the
/// 5-second dedup guard in `heartbeat::heartbeat()` does not suppress the beat.
#[test]
#[ignore]
fn heartbeat_delivers_to_pane_zero() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    // Set up: heartbeat enabled, last beat was 60 s ago (clears dedup guard).
    let old_ts = now_secs().saturating_sub(60);
    guard.write_state(&serde_json::json!({
        "last_beat_ts": old_ts,
        "interval_secs": 30,
        "last_sent": true,
        "next_beat_ts": 0,
        "needs_attention": false,
        "disabled": false
    }));

    let status = Command::new(bin())
        .arg("heartbeat")
        .status()
        .expect("failed to run superharness heartbeat");
    assert!(
        status.success(),
        "superharness heartbeat exited with non-zero status: {status}"
    );

    // Allow tmux to process the injected keystrokes.
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Capture the last 80 lines of pane %0 and look for the [HEARTBEAT] marker.
    let capture = Command::new("tmux")
        .args(["capture-pane", "-t", "%0", "-p", "-S", "-80"])
        .output()
        .expect("tmux capture-pane failed — is a tmux session running?");

    let pane_text = String::from_utf8_lossy(&capture.stdout);
    assert!(
        pane_text.contains("[HEARTBEAT]"),
        "expected [HEARTBEAT] in pane %0 output but not found.\n\
         Make sure a tmux session named 'superharness' exists with pane %0.\n\
         Captured pane content (last 80 lines):\n{pane_text}"
    );
}

/// When the heartbeat is disabled, `superharness heartbeat` must NOT send
/// a [HEARTBEAT] message to %0.
///
/// The test records the current %0 content before and after running the
/// command and verifies no new [HEARTBEAT] line was added.
#[test]
#[ignore]
fn heartbeat_skipped_when_disabled() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    // State: disabled=true, dedup guard would otherwise allow a beat.
    guard.write_state(&serde_json::json!({
        "last_beat_ts": now_secs().saturating_sub(60),
        "interval_secs": 30,
        "last_sent": true,
        "next_beat_ts": 0,
        "needs_attention": false,
        "disabled": true
    }));

    // Snapshot pane %0 before invoking the command.
    let before = Command::new("tmux")
        .args(["capture-pane", "-t", "%0", "-p", "-S", "-80"])
        .output()
        .expect("tmux capture-pane failed");
    let before_text = String::from_utf8_lossy(&before.stdout).to_string();
    let heartbeat_count_before = before_text.matches("[HEARTBEAT]").count();

    let status = Command::new(bin())
        .arg("heartbeat")
        .status()
        .expect("run superharness heartbeat");
    assert!(status.success());

    std::thread::sleep(std::time::Duration::from_millis(300));

    // Snapshot again; the count must not have increased.
    let after = Command::new("tmux")
        .args(["capture-pane", "-t", "%0", "-p", "-S", "-80"])
        .output()
        .expect("tmux capture-pane failed");
    let after_text = String::from_utf8_lossy(&after.stdout).to_string();
    let heartbeat_count_after = after_text.matches("[HEARTBEAT]").count();

    assert_eq!(
        heartbeat_count_after, heartbeat_count_before,
        "no new [HEARTBEAT] should appear in %0 when heartbeats are disabled"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Group 2 — heartbeat-toggle: set and clear the disabled flag
// ─────────────────────────────────────────────────────────────────────────────

/// Toggle from **enabled → disabled**: the state file must show `disabled=true`.
#[test]
#[ignore]
fn heartbeat_toggle_enabled_to_disabled() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    guard.write_state(&serde_json::json!({
        "last_beat_ts": 0,
        "interval_secs": 30,
        "last_sent": false,
        "next_beat_ts": 0,
        "needs_attention": false,
        "disabled": false
    }));

    let status = Command::new(bin())
        .arg("heartbeat-toggle")
        .status()
        .expect("run heartbeat-toggle");
    assert!(
        status.success(),
        "heartbeat-toggle exited non-zero: {status}"
    );

    let state = guard.read_state();
    assert_eq!(
        state["disabled"].as_bool(),
        Some(true),
        "disabled must be true after toggling off; full state: {state}"
    );
}

/// Toggle from **disabled → enabled**: `disabled=false` and `next_beat_ts`
/// must be set to a future Unix timestamp.
#[test]
#[ignore]
fn heartbeat_toggle_disabled_to_enabled() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    let before = now_secs();

    guard.write_state(&serde_json::json!({
        "last_beat_ts": 0,
        "interval_secs": 30,
        "last_sent": false,
        "next_beat_ts": 0,
        "needs_attention": false,
        "disabled": true
    }));

    let status = Command::new(bin())
        .arg("heartbeat-toggle")
        .status()
        .expect("run heartbeat-toggle");
    assert!(status.success());

    let state = guard.read_state();

    assert_eq!(
        state["disabled"].as_bool(),
        Some(false),
        "disabled must be false after toggling on; full state: {state}"
    );

    // Re-enabling must also reset the countdown to a future value.
    let next_beat = state["next_beat_ts"].as_u64().unwrap_or(0);
    assert!(
        next_beat > before,
        "next_beat_ts ({next_beat}) must be set to the future (> {before}) after re-enabling"
    );
}

/// Full round-trip: **enabled → disabled → enabled**.
///
/// Verifies that two successive toggles leave the system in the original state.
#[test]
#[ignore]
fn heartbeat_toggle_roundtrip() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    guard.write_state(&serde_json::json!({
        "last_beat_ts": 0,
        "interval_secs": 30,
        "last_sent": false,
        "next_beat_ts": 0,
        "needs_attention": false,
        "disabled": false
    }));

    // First toggle → disabled.
    Command::new(bin())
        .arg("heartbeat-toggle")
        .status()
        .expect("first toggle");
    assert_eq!(
        guard.read_state()["disabled"].as_bool(),
        Some(true),
        "should be disabled after first toggle"
    );

    // Second toggle → re-enabled.
    Command::new(bin())
        .arg("heartbeat-toggle")
        .status()
        .expect("second toggle");
    assert_eq!(
        guard.read_state()["disabled"].as_bool(),
        Some(false),
        "should be enabled after second toggle"
    );
}

/// `heartbeat-toggle` on a **fresh state file** (all defaults) must write
/// `disabled=true` — the first toggle always disables.
#[test]
#[ignore]
fn heartbeat_toggle_from_default_state_disables() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    // Write the default/zero state (mirrors HeartbeatState::default()).
    guard.write_state(&serde_json::json!({
        "last_beat_ts": 0,
        "interval_secs": 0,
        "last_sent": false,
        "next_beat_ts": 0,
        "needs_attention": false,
        "disabled": false
    }));

    Command::new(bin())
        .arg("heartbeat-toggle")
        .status()
        .expect("heartbeat-toggle");

    let state = guard.read_state();
    assert_eq!(
        state["disabled"].as_bool(),
        Some(true),
        "toggling on a default-state heartbeat must set disabled=true; state: {state}"
    );
}

/// Toggling back on after disabling must restore the `interval_secs` that was
/// already stored — it must not overwrite it with zero.
#[test]
#[ignore]
fn heartbeat_toggle_on_preserves_interval() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    guard.write_state(&serde_json::json!({
        "last_beat_ts": 0,
        "interval_secs": 45,
        "last_sent": false,
        "next_beat_ts": 0,
        "needs_attention": false,
        "disabled": true
    }));

    Command::new(bin())
        .arg("heartbeat-toggle")
        .status()
        .expect("heartbeat-toggle (re-enable)");

    let state = guard.read_state();
    assert_eq!(
        state["disabled"].as_bool(),
        Some(false),
        "must be enabled after toggle; state: {state}"
    );
    assert_eq!(
        state["interval_secs"].as_u64(),
        Some(45),
        "interval_secs must be preserved across toggle-on; state: {state}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Group 3 — heartbeat --snooze: push next_beat_ts forward
// ─────────────────────────────────────────────────────────────────────────────

/// `heartbeat --snooze 10` must advance `next_beat_ts` by exactly 10 seconds.
#[test]
#[ignore]
fn heartbeat_snooze_advances_next_beat_by_given_seconds() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    let baseline_next_beat = now_secs() + 30;
    guard.write_state(&serde_json::json!({
        "last_beat_ts": now_secs().saturating_sub(60),
        "interval_secs": 30,
        "last_sent": true,
        "next_beat_ts": baseline_next_beat,
        "needs_attention": false,
        "disabled": false
    }));

    let status = Command::new(bin())
        .args(["heartbeat", "--snooze", "10"])
        .status()
        .expect("run heartbeat --snooze 10");
    assert!(status.success());

    let state = guard.read_state();
    let new_next_beat = state["next_beat_ts"]
        .as_u64()
        .expect("next_beat_ts should be a u64");

    assert_eq!(
        new_next_beat,
        baseline_next_beat + 10,
        "snooze 10s must push next_beat_ts from {baseline_next_beat} to {}; got {new_next_beat}",
        baseline_next_beat + 10,
    );
}

/// Two sequential snooze calls are additive: 10 s + 5 s = 15 s total advance.
#[test]
#[ignore]
fn heartbeat_snooze_is_additive() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    let baseline = now_secs() + 30;
    guard.write_state(&serde_json::json!({
        "last_beat_ts": now_secs().saturating_sub(60),
        "interval_secs": 30,
        "last_sent": true,
        "next_beat_ts": baseline,
        "needs_attention": false,
        "disabled": false
    }));

    // First snooze: +10 s.
    Command::new(bin())
        .args(["heartbeat", "--snooze", "10"])
        .status()
        .unwrap();

    // Second snooze: +5 s more.
    Command::new(bin())
        .args(["heartbeat", "--snooze", "5"])
        .status()
        .unwrap();

    let state = guard.read_state();
    let new_next_beat = state["next_beat_ts"]
        .as_u64()
        .expect("next_beat_ts should be a u64");

    assert_eq!(
        new_next_beat,
        baseline + 15,
        "two snoozes of 10 s + 5 s must total 15 s advance; baseline={baseline}, got {new_next_beat}"
    );
}

/// `heartbeat --snooze 0` is a no-op: `next_beat_ts` must not change.
#[test]
#[ignore]
fn heartbeat_snooze_zero_is_noop() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    let baseline = now_secs() + 30;
    guard.write_state(&serde_json::json!({
        "last_beat_ts": now_secs().saturating_sub(60),
        "interval_secs": 30,
        "last_sent": true,
        "next_beat_ts": baseline,
        "needs_attention": false,
        "disabled": false
    }));

    Command::new(bin())
        .args(["heartbeat", "--snooze", "0"])
        .status()
        .expect("run heartbeat --snooze 0");

    let state = guard.read_state();
    let new_next_beat = state["next_beat_ts"]
        .as_u64()
        .expect("next_beat_ts should be a u64");

    assert_eq!(
        new_next_beat, baseline,
        "snooze 0 must not change next_beat_ts; baseline={baseline}, got {new_next_beat}"
    );
}

/// A large snooze (e.g. 3 600 s = 1 h) must not overflow or panic.
#[test]
#[ignore]
fn heartbeat_snooze_large_value_does_not_overflow() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    let baseline = now_secs() + 30;
    guard.write_state(&serde_json::json!({
        "last_beat_ts": now_secs().saturating_sub(60),
        "interval_secs": 30,
        "last_sent": true,
        "next_beat_ts": baseline,
        "needs_attention": false,
        "disabled": false
    }));

    let status = Command::new(bin())
        .args(["heartbeat", "--snooze", "3600"])
        .status()
        .expect("run heartbeat --snooze 3600");
    assert!(status.success(), "large snooze must not crash the binary");

    let state = guard.read_state();
    let new_next_beat = state["next_beat_ts"]
        .as_u64()
        .expect("next_beat_ts should be a u64");

    assert_eq!(
        new_next_beat,
        baseline + 3600,
        "snooze 3600 s must advance next_beat_ts by 1 h; baseline={baseline}, got {new_next_beat}"
    );
}

/// `heartbeat --snooze N` must **not** send any [HEARTBEAT] message to %0
/// (snooze mode only updates the state file).
#[test]
#[ignore]
fn heartbeat_snooze_does_not_send_heartbeat_to_pane_zero() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = ProjectGuard::new();

    let baseline = now_secs() + 30;
    guard.write_state(&serde_json::json!({
        "last_beat_ts": now_secs().saturating_sub(60),
        "interval_secs": 30,
        "last_sent": true,
        "next_beat_ts": baseline,
        "needs_attention": false,
        "disabled": false
    }));

    // Record how many [HEARTBEAT] lines are already in %0 before the snooze.
    let before = Command::new("tmux")
        .args(["capture-pane", "-t", "%0", "-p", "-S", "-80"])
        .output()
        .expect("tmux capture-pane before snooze");
    let before_count = String::from_utf8_lossy(&before.stdout)
        .matches("[HEARTBEAT]")
        .count();

    Command::new(bin())
        .args(["heartbeat", "--snooze", "10"])
        .status()
        .expect("run heartbeat --snooze 10");

    std::thread::sleep(std::time::Duration::from_millis(300));

    let after = Command::new("tmux")
        .args(["capture-pane", "-t", "%0", "-p", "-S", "-80"])
        .output()
        .expect("tmux capture-pane after snooze");
    let after_count = String::from_utf8_lossy(&after.stdout)
        .matches("[HEARTBEAT]")
        .count();

    assert_eq!(
        after_count, before_count,
        "snooze must not send any new [HEARTBEAT] to pane %0; \
         before={before_count}, after={after_count}"
    );
}
