use anyhow::Result;
use serde::Serialize;

use crate::monitor::{load_state, MonitorState};
use crate::tmux;
use crate::util::{hash_string, now_unix};

/// Health classification for a single pane.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Pane is at a shell prompt — not running anything.
    Idle,
    /// Output is actively changing between samples.
    Working,
    /// Output is unchanged and the pane is NOT at a prompt — likely blocked.
    Stalled,
    /// A permission / y-n prompt was detected — waiting for human approval.
    Waiting,
    /// A task-complete marker was found in output.
    Done,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            HealthStatus::Idle => "idle",
            HealthStatus::Working => "working",
            HealthStatus::Stalled => "stalled",
            HealthStatus::Waiting => "waiting",
            HealthStatus::Done => "done",
        };
        write!(f, "{s}")
    }
}

/// Health report for a single pane.
#[derive(Debug, Clone, Serialize)]
pub struct PaneHealth {
    /// Tmux pane ID (e.g. "%3").
    pub id: String,
    /// Classified health status.
    pub status: HealthStatus,
    /// How many seconds ago the output last changed (per monitor state).
    /// None if no prior monitor state exists for this pane.
    pub last_activity_ago: Option<u64>,
    /// Number of recovery attempts monitor has made on this pane.
    pub recovery_attempts: u32,
    /// True when the pane needs human attention (stalled after exhausting
    /// recovery, or waiting on a permission prompt).
    pub needs_attention: bool,
}

/// Return true if the pane is at a shell / REPL idle prompt.
fn is_idle_prompt(output: &str) -> bool {
    let last_line = output
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();

    last_line.ends_with('$')
        || last_line.ends_with('#')
        || last_line.ends_with('>')
        || last_line.ends_with('%')
}

/// Return true if the output contains a task-complete marker.
fn is_done(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("task complete")
        || lower.contains("task completed")
        || lower.contains("all done")
        || lower.contains("✓ done")
        || lower.contains("✅")
}

/// Return true if a permission / y-n prompt is visible.
///
/// Patterns matched (case-insensitive):
/// - `[y/n]`, `(y/n)`, `y/n?`, `yes/no`, `[yes/no]`
/// - `allow this?`, `approve?`, `confirm?`
/// - opencode-style `(Y/n)` / `(y/N)` prompts
fn is_waiting_for_permission(output: &str) -> bool {
    // Check the last few non-empty lines — permission prompts are usually at the end.
    let tail: String = output
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(5)
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
        || lower.contains("(y/n)")
        || lower.contains("[yes/no]")
        || lower.contains("(yes/no)")
        || lower.contains("yes/no?")
        || lower.contains("(y/n):")
        || lower.contains("yes or no")
    {
        return true;
    }

    // Mixed-case y/N or Y/n variants (case-insensitive already handled above)
    // opencode often shows: "Allow bash: ... (Y/n)"
    if lower.contains("(y/n") || lower.contains("[y/n") {
        return true;
    }

    // Generic approval keywords at end of a line
    if lower.contains("approve?")
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

/// Classify a single pane, merging live output with persisted monitor state.
pub fn classify_pane(
    pane_id: &str,
    monitor_state: &MonitorState,
    interval_secs: u64,
) -> Result<PaneHealth> {
    // Read current output (100 lines is enough for classification).
    let output = tmux::read(pane_id, 100)?;
    let current_hash = hash_string(&output);

    let prev_hash = monitor_state.last_output_hash.get(pane_id).copied();
    let recovery_attempts = monitor_state
        .recovery_attempts
        .get(pane_id)
        .copied()
        .unwrap_or(0);

    // Compute approximate "last activity ago" from stall counts + interval hint.
    // stall_count × interval_secs gives a lower bound on idle time.
    let stall_count = monitor_state
        .stall_counts
        .get(pane_id)
        .copied()
        .unwrap_or(0);
    let last_activity_ago = if stall_count > 0 {
        Some(stall_count as u64 * interval_secs)
    } else {
        // If output changed recently (or no prior data), report 0
        Some(0)
    };

    // --- Classify ---

    // 1. Check for permission/waiting prompt first — highest priority signal
    if is_waiting_for_permission(&output) {
        return Ok(PaneHealth {
            id: pane_id.to_string(),
            status: HealthStatus::Waiting,
            last_activity_ago,
            recovery_attempts,
            needs_attention: true,
        });
    }

    // 2. Done marker
    if is_done(&output) {
        return Ok(PaneHealth {
            id: pane_id.to_string(),
            status: HealthStatus::Done,
            last_activity_ago,
            recovery_attempts,
            needs_attention: false,
        });
    }

    // 3. Idle at shell prompt
    if is_idle_prompt(&output) {
        return Ok(PaneHealth {
            id: pane_id.to_string(),
            status: HealthStatus::Idle,
            last_activity_ago,
            recovery_attempts,
            needs_attention: false,
        });
    }

    // 4. Output changed since last snapshot → working
    if Some(current_hash) != prev_hash {
        return Ok(PaneHealth {
            id: pane_id.to_string(),
            status: HealthStatus::Working,
            last_activity_ago: Some(0),
            recovery_attempts,
            needs_attention: false,
        });
    }

    // 5. Output unchanged and NOT at a prompt → stalled
    let needs_attention = recovery_attempts >= 3;
    Ok(PaneHealth {
        id: pane_id.to_string(),
        status: HealthStatus::Stalled,
        last_activity_ago,
        recovery_attempts,
        needs_attention,
    })
}

/// Run a one-shot healthcheck and print JSON results.
///
/// `pane_filter` — if Some, only check that pane; otherwise check all panes.
/// `interval_hint` — used to estimate `last_activity_ago` from stall counts.
pub fn run(pane_filter: Option<&str>, interval_hint: u64) -> Result<()> {
    let monitor_state = load_state();

    // Collect pane IDs to check
    let pane_ids: Vec<String> = if let Some(pane) = pane_filter {
        vec![pane.to_string()]
    } else {
        tmux::list()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.id)
            .collect()
    };

    let timestamp = now_unix();

    if pane_ids.is_empty() {
        let out = serde_json::json!({
            "timestamp": timestamp,
            "panes": []
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let mut results: Vec<serde_json::Value> = Vec::new();

    for pane_id in &pane_ids {
        match classify_pane(pane_id, &monitor_state, interval_hint) {
            Ok(health) => {
                results.push(serde_json::json!({
                    "id": health.id,
                    "status": health.status.to_string(),
                    "last_activity_ago": health.last_activity_ago,
                    "recovery_attempts": health.recovery_attempts,
                    "needs_attention": health.needs_attention,
                }));
            }
            Err(e) => {
                // Include error panes as a special status so callers can see them
                results.push(serde_json::json!({
                    "id": pane_id,
                    "status": "error",
                    "error": e.to_string(),
                    "last_activity_ago": null,
                    "recovery_attempts": 0,
                    "needs_attention": true,
                }));
            }
        }
    }

    let out = serde_json::json!({
        "timestamp": timestamp,
        "panes": results,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
