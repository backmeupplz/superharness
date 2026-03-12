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
            }
        }
        Err(e) => PaneAction {
            pane: pane_id.to_string(),
            status: "done".to_string(),
            action: "kill_failed".to_string(),
            detail: Some(e.to_string()),
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
        };
    }

    // Safe prompt — send "y".
    match tmux::send(pane_id, "y") {
        Ok(_) => PaneAction {
            pane: pane_id.to_string(),
            status: "waiting".to_string(),
            action: "approved".to_string(),
            detail: Some("sent 'y' to safe permission prompt".to_string()),
        },
        Err(e) => PaneAction {
            pane: pane_id.to_string(),
            status: "waiting".to_string(),
            action: "approve_failed".to_string(),
            detail: Some(e.to_string()),
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
                    });
                    continue;
                }
            };

            let action = match health.status {
                HealthStatus::Done => {
                    // Reset stall counter on transition.
                    stall_counts.remove(pane_id.as_str());
                    handle_done(pane_id)
                }
                HealthStatus::Waiting => {
                    stall_counts.remove(pane_id.as_str());
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
                    }
                }
            };

            actions.push(action);
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
