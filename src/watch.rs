//! `superharness watch` — auto follow-up and review loop.
//!
//! Every `interval` seconds this loop:
//! 1. Runs a healthcheck on all worker panes (or a specific one).
//! 2. **Done** panes  → reads final output, kills the pane, triggers run-pending.
//! 3. **Waiting** panes → auto-approves safe permission prompts (sends "y").
//!    If the prompt looks destructive, prints a warning but does NOT approve.
//! 4. **Stalled** panes → escalating follow-up:
//!    - 1st stall  → "please continue"
//!    - 2nd stall  → "are you stuck? what do you need?"
//!    - 3rd+ stall → marks as needs_attention, stops nudging.
//! 5. Prints a JSON status update each cycle.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::events;
use crate::health::{classify_pane, HealthStatus};
use crate::layout;
use crate::monitor::load_state;
use crate::pending_tasks;
use crate::project;
use crate::relay;
use crate::tmux;
use crate::tmux::smart_layout_with_attention;

// ---------------------------------------------------------------------------
// Destructive-pattern detection
// ---------------------------------------------------------------------------

/// Returns true when the permission prompt text contains patterns that are
/// considered destructive / irreversible and should NOT be auto-approved.
fn is_destructive_prompt(output: &str) -> bool {
    let lower = output.to_lowercase();

    // File-system nukes
    if lower.contains("rm -rf")
        || lower.contains("rm -fr")
        || lower.contains("rmdir")
        || lower.contains("shred")
    {
        return true;
    }

    // Git force operations
    if lower.contains("force push")
        || lower.contains("push --force")
        || lower.contains("push -f ")
        || lower.contains("git push -f")
    {
        return true;
    }

    // Database operations
    if lower.contains("drop database")
        || lower.contains("drop table")
        || lower.contains("truncate table")
    {
        return true;
    }

    // Cloud / infra deletions
    if lower.contains("delete cluster")
        || lower.contains("destroy ")
        || lower.contains("terraform destroy")
    {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Per-pane stall tracking (in-process — not persisted across watch restarts)
// ---------------------------------------------------------------------------

/// Number of consecutive stall cycles detected per pane.
type StallCounts = HashMap<String, u32>;

// ---------------------------------------------------------------------------
// Action record — what the watch loop did to a pane in one cycle
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PaneAction {
    pub pane: String,
    pub status: String,
    pub action: String,
    pub detail: Option<String>,
    /// True when this pane was surfaced to the main window during this cycle.
    pub surfaced: bool,
}

// ---------------------------------------------------------------------------
// Pulse — orchestrator heartbeat digest
// ---------------------------------------------------------------------------

/// Result from a pulse operation.
#[derive(Debug, Serialize)]
pub struct PulseResult {
    /// Whether the digest was actually sent to %0.
    pub sent: bool,
    /// Target pane (always "%0").
    pub target_pane: String,
    /// The full message that was sent, if sent.
    pub message: Option<String>,
    /// Number of worker panes inspected.
    pub worker_count: usize,
    /// Reason the pulse was skipped (if `sent` is false).
    pub reason_skipped: Option<String>,
}

/// Build a [PULSE] digest of all worker panes and optionally send it to %0.
///
/// `force_send` — when `true`, always send if at least one worker exists
///               (used by the standalone `pulse` subcommand).
///               When `false`, only send when at least one worker is in an
///               actionable state (done, waiting, stalled, or error); used
///               by the watch loop.
pub fn pulse(force_send: bool) -> Result<PulseResult> {
    let monitor_state = load_state();

    // List all panes, skip %0 (the orchestrator itself).
    let all_panes = tmux::list().unwrap_or_default();
    let worker_panes: Vec<_> = all_panes.iter().filter(|p| p.id != "%0").collect();

    if worker_panes.is_empty() {
        return Ok(PulseResult {
            sent: false,
            target_pane: "%0".to_string(),
            message: None,
            worker_count: 0,
            reason_skipped: Some("no active workers".to_string()),
        });
    }

    let mut summaries: Vec<String> = Vec::new();
    let mut actions_needed: Vec<String> = Vec::new();
    let mut has_actionable = false;

    for pane in &worker_panes {
        // Use pane title as a short label; fall back to ID.
        let label: String = if pane.title.is_empty() {
            pane.id.clone()
        } else {
            pane.title.chars().take(24).collect()
        };

        match classify_pane(&pane.id, &monitor_state, 60) {
            Ok(health) => {
                let status_str = match health.status {
                    HealthStatus::Working => "working".to_string(),
                    HealthStatus::Idle => "idle".to_string(),
                    HealthStatus::Stalled => {
                        has_actionable = true;
                        "STALLED".to_string()
                    }
                    HealthStatus::Waiting => {
                        has_actionable = true;
                        "WAITING approval".to_string()
                    }
                    HealthStatus::Done => {
                        has_actionable = true;
                        "done".to_string()
                    }
                };

                summaries.push(format!("{} {} ({})", pane.id, status_str, label));

                // Collect explicit action items.
                match health.status {
                    HealthStatus::Waiting => {
                        actions_needed.push(format!("approve {}", pane.id));
                    }
                    HealthStatus::Done => {
                        actions_needed.push(format!("collect {} output", pane.id));
                    }
                    HealthStatus::Stalled if health.needs_attention => {
                        actions_needed.push(format!("check {} (stalled)", pane.id));
                    }
                    _ => {}
                }
            }
            Err(e) => {
                has_actionable = true;
                summaries.push(format!("{} ERROR ({}): {}", pane.id, label, e));
                actions_needed.push(format!("check {} (error)", pane.id));
            }
        }
    }

    // In non-forced mode, skip sending when everything is just working/idle.
    if !force_send && !has_actionable {
        return Ok(PulseResult {
            sent: false,
            target_pane: "%0".to_string(),
            message: None,
            worker_count: worker_panes.len(),
            reason_skipped: Some("no actionable workers (all working/idle)".to_string()),
        });
    }

    // Build the one-line digest.
    let worker_list = summaries.join(", ");
    let action_part = if actions_needed.is_empty() {
        "No immediate action needed.".to_string()
    } else {
        format!("Action needed: {}.", actions_needed.join(", "))
    };
    let message = format!(
        "[PULSE] {} worker(s) active: {}. {}",
        worker_panes.len(),
        worker_list,
        action_part
    );

    // Send to the orchestrator pane (%0).
    tmux::send("%0", &message)?;

    Ok(PulseResult {
        sent: true,
        target_pane: "%0".to_string(),
        message: Some(message),
        worker_count: worker_panes.len(),
        reason_skipped: None,
    })
}

// ---------------------------------------------------------------------------
// Heartbeat — unconditional periodic message to keep %0 active
// ---------------------------------------------------------------------------

/// Interval between heartbeat messages (seconds).
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

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
    // Skip if %0 looks busy to avoid interrupting an active response.
    if orchestrator_is_busy() {
        eprintln!("[watch] heartbeat skipped — %0 appears to be actively processing");
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
}

/// Return the path to the heartbeat state file.
/// Stored alongside events.json in the project-local .superharness/ directory.
pub fn heartbeat_state_path() -> std::path::PathBuf {
    project::get_project_state_dir()
        .map(|d| d.join("heartbeat_state.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/superharness-heartbeat-state.json"))
}

/// Write heartbeat state to disk after every beat attempt.
pub fn write_heartbeat_state(
    last_beat_ts: u64,
    interval_secs: u64,
    sent: bool,
    needs_attention: bool,
) {
    let state = HeartbeatState {
        last_beat_ts,
        interval_secs,
        last_sent: sent,
        next_beat_ts: last_beat_ts + interval_secs,
        needs_attention,
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
// Core logic helpers
// ---------------------------------------------------------------------------

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Handle a pane classified as **Done**: read output, kill pane, run-pending.
fn handle_done(pane_id: &str) -> PaneAction {
    // Capture final output for the log (last 50 lines).
    let final_output = tmux::read(pane_id, 50).unwrap_or_default();
    let last_line: String = final_output
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(120)
        .collect();

    // If the pane is currently visible in the main window, hide it to a
    // background tab first.  This lets the main window re-layout cleanly
    // before the pane is destroyed (a direct kill of a foreground pane can
    // leave the layout in an awkward state).
    if pane_is_in_main_window(pane_id) {
        eprintln!("[watch] handle_done: pane {pane_id} is in main window — hiding before kill");
        if let Err(e) = tmux::hide(pane_id, Some(pane_id)) {
            eprintln!("[watch] handle_done: failed to hide {pane_id}: {e}");
        } else {
            // Re-apply layout and enforce min size now that the done pane is gone.
            let _ = tmux::smart_layout();
            let _ = layout::enforce_min_pane_size();
        }
    }

    // Kill the pane.
    match tmux::kill(pane_id) {
        Ok(_) => {
            // Trigger run-pending so dependency-gated tasks can start.
            let active_panes: Vec<String> = tmux::list()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.id)
                .collect();

            let spawned = pending_tasks::ready_tasks(&active_panes)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|t| {
                    match tmux::spawn(
                        &t.task,
                        &t.dir,
                        t.name.as_deref(),
                        t.model.as_deref(),
                        t.harness.as_deref(),
                        t.mode.as_deref(),
                        false, // auto-hide by default
                    ) {
                        Ok(new_pane) => {
                            let _ = pending_tasks::remove_task(&t.id);
                            Some(new_pane) // show in main window (default)
                        }
                        Err(_) => None,
                    }
                })
                .collect::<Vec<_>>();

            let detail = if spawned.is_empty() {
                format!("killed; last_line={last_line:?}")
            } else {
                format!(
                    "killed; triggered run-pending → spawned {:?}; last_line={last_line:?}",
                    spawned
                )
            };

            let _ = tmux::flash_notification(&format!("✓ Worker {pane_id} done"));
            let _ = pulse(true);
            // Surface the orchestrator window so the user sees %0 after a worker finishes.
            let _ = tmux::select_orchestrator();
            let _ = events::log_event(events::EventKind::WorkerCompleted, Some(pane_id), &detail);

            PaneAction {
                pane: pane_id.to_string(),
                status: "done".to_string(),
                action: "killed".to_string(),
                detail: Some(detail),
                surfaced: false,
            }
        }
        Err(e) => PaneAction {
            pane: pane_id.to_string(),
            status: "done".to_string(),
            action: "kill_failed".to_string(),
            detail: Some(e.to_string()),
            surfaced: false,
        },
    }
}

/// Handle a pane classified as **Waiting** (permission prompt).
fn handle_waiting(pane_id: &str) -> PaneAction {
    // Read the last 10 lines to inspect the prompt.
    let output = tmux::read(pane_id, 10).unwrap_or_default();

    if is_destructive_prompt(&output) {
        eprintln!(
            "[watch] WARNING: pane {pane_id} has a DESTRUCTIVE permission prompt — NOT auto-approving. Manual review required."
        );
        return PaneAction {
            pane: pane_id.to_string(),
            status: "waiting".to_string(),
            action: "skipped_destructive".to_string(),
            detail: Some("destructive pattern detected — manual approval required".to_string()),
            surfaced: false,
        };
    }

    // Safe prompt — send "y".
    match tmux::send(pane_id, "y") {
        Ok(_) => PaneAction {
            pane: pane_id.to_string(),
            status: "waiting".to_string(),
            action: "approved".to_string(),
            detail: Some("sent 'y' to safe permission prompt".to_string()),
            surfaced: false,
        },
        Err(e) => PaneAction {
            pane: pane_id.to_string(),
            status: "waiting".to_string(),
            action: "approve_failed".to_string(),
            detail: Some(e.to_string()),
            surfaced: false,
        },
    }
}

/// Handle a pane classified as **Stalled**, using escalating nudges.
fn handle_stalled(pane_id: &str, stall_counts: &mut StallCounts) -> PaneAction {
    let count = stall_counts.entry(pane_id.to_string()).or_insert(0);
    *count += 1;
    let stall_number = *count;

    match stall_number {
        1 => {
            let msg = "please continue";
            let _ = tmux::send(pane_id, msg);
            let _ = tmux::flash_notification(&format!("⚠ Worker {pane_id} stalled — nudging"));
            PaneAction {
                pane: pane_id.to_string(),
                status: "stalled".to_string(),
                action: "nudged".to_string(),
                detail: Some(format!("stall #{stall_number}: sent {msg:?}")),
                surfaced: false,
            }
        }
        2 => {
            let msg = "are you stuck? what do you need?";
            let _ = tmux::send(pane_id, msg);
            PaneAction {
                pane: pane_id.to_string(),
                status: "stalled".to_string(),
                action: "nudged".to_string(),
                detail: Some(format!("stall #{stall_number}: sent {msg:?}")),
                surfaced: false,
            }
        }
        _ => {
            eprintln!(
                "[watch] pane {pane_id} stalled for {stall_number} consecutive cycles — marked needs_attention"
            );
            PaneAction {
                pane: pane_id.to_string(),
                status: "stalled".to_string(),
                action: "needs_attention".to_string(),
                detail: Some(format!(
                    "stall #{stall_number}: exhausted nudges — human intervention required"
                )),
                surfaced: false,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pane window membership helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `pane_id` is currently in the main window (window 0)
/// of the superharness session.
fn pane_is_in_main_window(pane_id: &str) -> bool {
    let output = std::process::Command::new("tmux")
        .args(["list-panes", "-t", "superharness:0", "-F", "#{pane_id}"])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .any(|l| l.trim() == pane_id),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Relay-request surfacing
// ---------------------------------------------------------------------------

/// Check relay_requests.json for pending requests from any pane.
/// For each pending request:
///   1. Surface the requesting pane to the main window so the human sees it.
///   2. Send a [RELAY REQUEST] message to %0 describing the request and how
///      to answer it.
///
/// Returns the number of pending relay requests notified this cycle.
fn handle_pending_relays() -> usize {
    let pending = match relay::get_pending_relays() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[watch] failed to read relay requests: {e}");
            return 0;
        }
    };

    if pending.is_empty() {
        return 0;
    }

    let mut notified = 0;
    for req in &pending {
        // Surface the worker pane so the human can see what it is doing.
        if let Err(e) = tmux::surface(&req.pane_id) {
            eprintln!(
                "[watch] failed to surface pane {} for relay {}: {e}",
                req.pane_id, req.id
            );
        }

        // Build the message to send to %0.
        let sens_note = if req.sensitive {
            " [SENSITIVE — answer will not be logged]"
        } else {
            ""
        };
        let kind_tag = match req.kind {
            relay::RelayKind::Sudo => "[SUDO RELAY REQUEST]",
            relay::RelayKind::Question => "[RELAY REQUEST]",
        };
        let mut msg = format!(
            "{kind_tag} from pane {pane}{sens}\nQuestion: {q}",
            kind_tag = kind_tag,
            pane = req.pane_id,
            sens = sens_note,
            q = req.question,
        );
        if !req.context.is_empty() {
            msg.push_str(&format!("\nContext: {}", req.context));
        }
        msg.push_str(&format!(
            "\n\nAnswer with: superharness relay-answer --id {id} --answer \"<value>\"",
            id = req.id
        ));

        if let Err(e) = tmux::send("%0", &msg) {
            eprintln!(
                "[watch] failed to send relay notification to %0 for {}: {e}",
                req.id
            );
        } else {
            notified += 1;
            eprintln!(
                "[watch] relayed request {} from pane {} to orchestrator",
                req.id, req.pane_id
            );
        }
    }

    notified
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

// ---------------------------------------------------------------------------
// Watch loop
// ---------------------------------------------------------------------------

/// Run the watch loop.
///
/// `interval_secs`  — seconds between cycles (default 30).
/// `pane_filter`    — if Some, only watch that pane; otherwise watch all panes.
pub fn run(interval_secs: u64, pane_filter: Option<&str>) -> Result<()> {
    println!(
        "Starting watch (interval={}s, pane={:?})",
        interval_secs, pane_filter
    );

    // In-process stall counters — reset when the pane transitions away from stalled.
    let mut stall_counts: StallCounts = HashMap::new();
    // Timestamp of the last heartbeat sent to %0.  Initialise to 0 so that
    // the very first cycle always fires a heartbeat immediately.
    let mut last_heartbeat: u64 = 0;
    // Timestamp of the last time a heartbeat was *actually delivered* to %0.
    // Used for the maximum-skip guard: if too much time has passed without a
    // delivery (even when %0 appears busy), we force-send anyway.
    let mut last_forced_heartbeat: u64 = 0;
    // Track the last observed terminal size so we can react to resize events.
    let mut last_terminal_size: Option<(u32, u32)> = None;
    // Track how many consecutive watch cycles each pane has been in "waiting" state.
    let mut waiting_cycles: HashMap<String, u32> = HashMap::new();

    loop {
        let cycle_ts = now_unix();
        let monitor_state = load_state();

        // ── Terminal resize detection ─────────────────────────────────────────
        // If the terminal grew or shrank significantly, re-apply the layout and
        // enforce the minimum pane size so nothing becomes unreadably small.
        if let Some((w, h)) = tmux::terminal_size() {
            let should_relayout = match last_terminal_size {
                None => false, // first cycle — no previous size to compare
                Some((lw, lh)) => {
                    let dw = if w > lw { w - lw } else { lw - w };
                    let dh = if h > lh { h - lh } else { lh - h };
                    dw > 5 || dh > 3
                }
            };
            if should_relayout {
                eprintln!("[RESIZE] terminal changed to {w}x{h}, re-applying layout");
                let _ = tmux::smart_layout();
                let _ = layout::enforce_min_pane_size();
            }
            last_terminal_size = Some((w, h));
        }

        // Collect pane IDs to inspect.
        let pane_ids: Vec<String> = if let Some(pane) = pane_filter {
            vec![pane.to_string()]
        } else {
            tmux::list()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.id)
                .collect()
        };

        let mut actions: Vec<PaneAction> = Vec::new();

        for pane_id in &pane_ids {
            let health = match classify_pane(pane_id, &monitor_state, interval_secs) {
                Ok(h) => h,
                Err(e) => {
                    actions.push(PaneAction {
                        pane: pane_id.clone(),
                        status: "error".to_string(),
                        action: "skipped".to_string(),
                        detail: Some(e.to_string()),
                        surfaced: false,
                    });
                    continue;
                }
            };

            let mut action = match health.status {
                HealthStatus::Done => {
                    // Reset stall and waiting counters on transition.
                    stall_counts.remove(pane_id.as_str());
                    waiting_cycles.remove(pane_id.as_str());
                    handle_done(pane_id)
                }
                HealthStatus::Waiting => {
                    stall_counts.remove(pane_id.as_str());

                    // Track consecutive waiting cycles for this pane.
                    let wc = waiting_cycles.entry(pane_id.clone()).or_insert(0);
                    *wc += 1;
                    let cycles_waiting = *wc;

                    // If the pane has been waiting for more than 2 consecutive cycles
                    // and is currently in a background tab, surface it automatically
                    // so the human can see it without manual hunting.
                    if cycles_waiting > 2 && !pane_is_in_main_window(pane_id) {
                        eprintln!(
                            "[watch] pane {pane_id} has been waiting for {cycles_waiting} cycles \
                             and is in background — surfacing automatically"
                        );
                        match tmux::surface(pane_id) {
                            Ok(_) => {
                                let _ = layout::enforce_min_pane_size();
                            }
                            Err(e) => eprintln!("[watch] failed to surface {pane_id}: {e}"),
                        }
                    }

                    // Surface and expand the waiting pane so the orchestrator
                    // can see it clearly without manual hunting.
                    match smart_layout_with_attention(Some(pane_id)) {
                        Ok(_) => eprintln!(
                            "[watch] smart_layout_with_attention({pane_id}): attention layout applied"
                        ),
                        Err(e) => eprintln!(
                            "[watch] smart_layout_with_attention({pane_id}) failed: {e}"
                        ),
                    }
                    handle_waiting(pane_id)
                }
                HealthStatus::Stalled => {
                    waiting_cycles.remove(pane_id.as_str());
                    handle_stalled(pane_id, &mut stall_counts)
                }
                HealthStatus::Working | HealthStatus::Idle => {
                    // Reset stall and waiting counters when pane becomes active again.
                    stall_counts.remove(pane_id.as_str());
                    waiting_cycles.remove(pane_id.as_str());
                    PaneAction {
                        pane: pane_id.clone(),
                        status: health.status.to_string(),
                        action: "observed".to_string(),
                        detail: None,
                        surfaced: false,
                    }
                }
            };

            // --- Autonomous pane management ---

            // Rule 1: Surface panes that need immediate human attention so the
            // human can see them in the main window without having to hunt for them.
            if action.action == "skipped_destructive" || action.action == "needs_attention" {
                match tmux::surface(pane_id) {
                    Ok(_) => {
                        action.surfaced = true;
                        eprintln!(
                            "[watch] surfaced pane {pane_id} to main window (action={})",
                            action.action
                        );
                    }
                    Err(e) => {
                        eprintln!("[watch] failed to surface pane {pane_id}: {e}");
                    }
                }
            }

            actions.push(action);
        }

        // Rule 2 & 3: Keep the main window tidy after every cycle.
        // auto_compact() moves excess worker panes (beyond MAX_WORKERS_VISIBLE=4) to
        // background tabs.  This also handles the case where several working/idle panes
        // are accumulating in the main window.
        let _ = tmux::auto_compact();

        // Check for pending relay requests from any pane and forward them to %0.
        let relay_notified = handle_pending_relays();
        if relay_notified > 0 {
            eprintln!(
                "[watch] forwarded {relay_notified} pending relay request(s) to orchestrator"
            );
        }

        // At the end of each cycle, send a [PULSE] digest to the orchestrator
        // pane (%0) when at least one worker had a non-trivial result this cycle
        // (i.e. something other than plain "observed").  This keeps the
        // orchestrator informed without flooding it when everything is idle.
        let cycle_has_actionable = actions
            .iter()
            .any(|a| a.action != "observed" && a.action != "skipped");
        if cycle_has_actionable {
            match pulse(false) {
                Ok(ref pr) if pr.sent => {
                    eprintln!(
                        "[watch] sent [PULSE] to {}: {:?}",
                        pr.target_pane,
                        pr.message.as_deref().unwrap_or("")
                    );
                }
                Ok(_) => {} // nothing actionable in pulse's own check — skip
                Err(e) => eprintln!("[watch] pulse error: {e}"),
            }
        }

        // Print JSON status update for this cycle.
        let out = serde_json::json!({
            "timestamp": cycle_ts,
            "cycle_actions": actions,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);

        // Unconditional timed heartbeat — fires every HEARTBEAT_INTERVAL_SECS regardless
        // of worker state so that the orchestrator pane (%0) never goes idle.
        // Skipped automatically when %0 is actively processing.
        //
        // Maximum-skip guard: if more than 2 × HEARTBEAT_INTERVAL_SECS have passed
        // since the last *delivered* heartbeat, force-send even if %0 looks busy.
        // This prevents the heartbeat from being suppressed indefinitely.
        if cycle_ts.saturating_sub(last_heartbeat) >= HEARTBEAT_INTERVAL_SECS {
            let force =
                cycle_ts.saturating_sub(last_forced_heartbeat) > 2 * HEARTBEAT_INTERVAL_SECS;

            // Pre-compute needs_attention for state file regardless of send outcome.
            let hb_all_panes = tmux::list().unwrap_or_default();
            let hb_workers: Vec<_> = hb_all_panes.iter().filter(|p| p.id != "%0").collect();
            let hb_monitor = load_state();
            let hb_needs_attention = hb_workers.iter().any(|p| {
                matches!(
                    classify_pane(&p.id, &hb_monitor, 60),
                    Ok(h) if matches!(h.status, HealthStatus::Stalled | HealthStatus::Waiting)
                )
            });

            let sent = if force {
                // Force-send: bypass the busy check.
                eprintln!("[watch] [HEARTBEAT] force-sending — max-skip guard triggered");
                let worker_count = hb_workers.len();
                let time = time_hhmm();
                let status_part = if worker_count == 0 {
                    "No workers running".to_string()
                } else if hb_needs_attention {
                    let cnt = hb_workers
                        .iter()
                        .filter(|p| {
                            matches!(
                                classify_pane(&p.id, &hb_monitor, 60),
                                Ok(h) if matches!(
                                    h.status,
                                    HealthStatus::Stalled | HealthStatus::Waiting
                                )
                            )
                        })
                        .count();
                    format!("{cnt} worker(s) need attention")
                } else {
                    "All systems nominal".to_string()
                };
                let msg = format!(
                    "[HEARTBEAT] Active workers: {worker_count} | Time: {time} | {status_part}"
                );
                let ok = tmux::send("%0", &msg).is_ok();
                if ok {
                    let _ = tmux::flash_notification(&msg);
                }
                ok
            } else {
                match heartbeat() {
                    Ok(true) => {
                        eprintln!("[watch] sent [HEARTBEAT] to %0");
                        true
                    }
                    Ok(false) => {
                        eprintln!(
                            "[watch] [HEARTBEAT] skipped — %0 is busy, will retry next cycle"
                        );
                        false
                    }
                    Err(e) => {
                        eprintln!("[watch] heartbeat error: {e}");
                        true // treat errors as "sent" to avoid retry storms
                    }
                }
            };

            // Write heartbeat state to disk (sent or skipped).
            write_heartbeat_state(cycle_ts, HEARTBEAT_INTERVAL_SECS, sent, hb_needs_attention);

            last_heartbeat = cycle_ts;
            if sent {
                last_forced_heartbeat = cycle_ts;
            }
        }

        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}
