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
use serde::Serialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::health::{classify_pane, HealthStatus};
use crate::monitor::load_state;
use crate::pending_tasks;
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
                        t.mode.as_deref(),
                    ) {
                        Ok(new_pane) => {
                            let _ = pending_tasks::remove_task(&t.id);
                            Some(new_pane)
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

    loop {
        let cycle_ts = now_unix();
        let monitor_state = load_state();

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
                    // Reset stall counter on transition.
                    stall_counts.remove(pane_id.as_str());
                    handle_done(pane_id)
                }
                HealthStatus::Waiting => {
                    stall_counts.remove(pane_id.as_str());
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
                HealthStatus::Stalled => handle_stalled(pane_id, &mut stall_counts),
                HealthStatus::Working | HealthStatus::Idle => {
                    // Reset stall counter when pane becomes active again.
                    stall_counts.remove(pane_id.as_str());
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

        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}
