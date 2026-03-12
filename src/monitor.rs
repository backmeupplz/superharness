use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

use crate::tmux;

/// State persisted between monitor runs.
#[derive(Serialize, Deserialize, Default)]
pub struct MonitorState {
    /// Number of consecutive checks where output was unchanged per pane.
    pub stall_counts: HashMap<String, u32>,
    /// Hash of the last seen output per pane.
    pub last_output_hash: HashMap<String, u64>,
    /// Number of recovery attempts already made per pane.
    pub recovery_attempts: HashMap<String, u32>,
}

fn state_path() -> PathBuf {
    let base = dirs_home().unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(".local")
        .join("share")
        .join("superharness")
        .join("monitor_state.json")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

fn load_state() -> MonitorState {
    let path = state_path();
    if !path.exists() {
        return MonitorState::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => MonitorState::default(),
    }
}

fn save_state(state: &MonitorState) -> Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create state directory: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write monitor state: {}", path.display()))?;
    Ok(())
}

fn hash_output(output: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    output.hash(&mut hasher);
    hasher.finish()
}

/// Returns true if the pane output looks like it is at a resting prompt
/// or completed state (i.e., NOT stalled — waiting for user intentionally).
fn looks_like_prompt_or_complete(output: &str) -> bool {
    // Grab the last non-empty line for inspection
    let last_line = output
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();

    // Common shell / REPL prompts — not a stall
    if last_line.ends_with('$')
        || last_line.ends_with('#')
        || last_line.ends_with('>')
        || last_line.ends_with('%')
    {
        return true;
    }

    // opencode / AI tool completion markers
    let lower = last_line.to_lowercase();
    if lower.contains("task complete")
        || lower.contains("task completed")
        || lower.contains("all done")
        || lower.contains("finished")
        || lower.contains("waiting for input")
        || lower.contains("press enter")
    {
        return true;
    }

    false
}

/// Perform one check cycle on a single pane.
/// Returns a human-readable status message for the pane.
fn check_pane(pane_id: &str, state: &mut MonitorState, stall_threshold: u32) -> String {
    let output = match tmux::read(pane_id, 100) {
        Ok(o) => o,
        Err(e) => {
            return format!("[{}] ERROR reading pane: {}", pane_id, e);
        }
    };

    let hash = hash_output(&output);
    let prev_hash = state.last_output_hash.get(pane_id).copied();

    // Update stored hash
    state.last_output_hash.insert(pane_id.to_string(), hash);

    if Some(hash) != prev_hash {
        // Output has changed — reset stall counter
        state.stall_counts.insert(pane_id.to_string(), 0);
        state.recovery_attempts.insert(pane_id.to_string(), 0);
        return format!("[{}] active — output changed", pane_id);
    }

    // Output unchanged — increment stall counter
    let stall_count = state.stall_counts.entry(pane_id.to_string()).or_insert(0);
    *stall_count += 1;
    let current_stall = *stall_count;

    if current_stall < stall_threshold {
        return format!(
            "[{}] unchanged for {}/{} checks — watching",
            pane_id, current_stall, stall_threshold
        );
    }

    // We are at or past the stall threshold — check if this is intentional
    if looks_like_prompt_or_complete(&output) {
        return format!(
            "[{}] output unchanged but ends at prompt/complete — not stalled",
            pane_id
        );
    }

    // Genuine stall — attempt recovery
    let attempts = state
        .recovery_attempts
        .entry(pane_id.to_string())
        .or_insert(0);
    let attempt = *attempts;
    *attempts += 1;

    match attempt {
        0 => {
            eprintln!("[monitor] [{}] STALLED — attempt 1: sending Enter", pane_id);
            let _ = tmux::send_raw(pane_id, "");
            format!("[{}] STALLED — sent Enter (recovery attempt 1)", pane_id)
        }
        1 => {
            eprintln!(
                "[monitor] [{}] STALLED — attempt 2: sending 'continue'",
                pane_id
            );
            let _ = tmux::send(pane_id, "continue");
            format!(
                "[{}] STALLED — sent 'continue' (recovery attempt 2)",
                pane_id
            )
        }
        2 => {
            eprintln!(
                "[monitor] [{}] STALLED — attempt 3: sending 'please continue with the task'",
                pane_id
            );
            let _ = tmux::send(pane_id, "please continue with the task");
            format!(
                "[{}] STALLED — sent detailed prompt (recovery attempt 3)",
                pane_id
            )
        }
        _ => {
            eprintln!(
                "[monitor] [{}] STALLED — all recovery attempts exhausted — needs human attention",
                pane_id
            );
            format!(
                "[{}] STALLED — exhausted recovery attempts — NEEDS HUMAN ATTENTION",
                pane_id
            )
        }
    }
}

/// Run the monitor loop.
pub fn run(interval_secs: u64, pane_filter: Option<&str>, stall_threshold: u32) -> Result<()> {
    println!(
        "Starting monitor (interval={}s, stall_threshold={} checks, pane={:?})",
        interval_secs, stall_threshold, pane_filter
    );

    loop {
        let mut state = load_state();

        // Collect panes to check
        let pane_ids: Vec<String> = if let Some(pane) = pane_filter {
            vec![pane.to_string()]
        } else {
            tmux::list()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.id)
                .collect()
        };

        if pane_ids.is_empty() {
            println!("[monitor] no panes found — waiting...");
        } else {
            for pane_id in &pane_ids {
                let status = check_pane(pane_id, &mut state, stall_threshold);
                println!("{}", status);
            }
        }

        if let Err(e) = save_state(&state) {
            eprintln!("[monitor] warning: could not save state: {}", e);
        }

        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}
