use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::{BOLD, BRIGHT_RED, CYAN, DIM, GREEN, RED, RESET, UNDERLINE, YELLOW};
use crate::{health, heartbeat, monitor, project, tmux};

/// Filter predicate: exclude the orchestrator pane so only real worker panes
/// are shown / counted.
fn is_worker_pane(p: &tmux::PaneInfo) -> bool {
    p.id != tmux::orchestrator_pane_id()
}

/// Handle `Command::StatusHuman` — human-readable mode + worker health display.
pub fn handle_status_human() -> Result<()> {
    // Read mode from project state file
    let state_dir = project::get_project_state_dir()?;
    let state_file = state_dir.join("state.json");
    let (mode_str, away_since, away_message) = if state_file.exists() {
        let content = std::fs::read_to_string(&state_file).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        let mode = v["mode"].as_str().unwrap_or("present").to_string();
        let since = v["away_since"].as_u64();
        let msg = v["instructions"]
            .as_str()
            .or_else(|| v["away_message"].as_str())
            .map(|s| s.to_string());
        (mode, since, msg)
    } else {
        ("present".to_string(), None, None)
    };

    // ── Hint bar ─────────────────────────────────────────────────────────────
    println!("  {DIM}any key to close{RESET}");
    println!("  {DIM}{}{RESET}", "─".repeat(70));

    // ── MODE ──────────────────────────────────────────────────────────────────
    println!();
    if mode_str == "away" {
        let away_since_str = away_since.map(|ts| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let elapsed = now.saturating_sub(ts);
            let h = elapsed / 3600;
            let m = (elapsed % 3600) / 60;
            format!("{h}h {m}m ago (since unix:{ts})")
        });
        println!("  {BOLD}{YELLOW}Mode:{RESET}    {BOLD}{YELLOW}AWAY{RESET}");
        if let Some(since) = away_since_str {
            println!("  {DIM}Away:{RESET}    {since}");
        }
        if let Some(ref msg) = away_message {
            println!("  {DIM}Message:{RESET} {msg}");
        }
    } else {
        println!("  {BOLD}{GREEN}Mode:{RESET}    {BOLD}{GREEN}PRESENT{RESET}");
    }

    // ── PENDING DECISIONS ─────────────────────────────────────────────────────
    let decisions_file = state_dir.join("decisions.json");
    println!();
    println!("  {BOLD}{UNDERLINE}Pending Decisions{RESET}");
    if decisions_file.exists() {
        let content = std::fs::read_to_string(&decisions_file).unwrap_or_default();
        let decisions: Vec<serde_json::Value> = serde_json::from_str(&content).unwrap_or_default();
        if decisions.is_empty() {
            println!("    {DIM}none{RESET}");
        } else {
            println!("    {BOLD}{}{RESET} decision(s) queued", decisions.len());
            for (i, d) in decisions.iter().enumerate() {
                println!();
                let pane = d["pane"].as_str().unwrap_or("?");
                let question = d["question"].as_str().unwrap_or("?");
                let context = d["context"].as_str().unwrap_or("");
                println!("    {BOLD}[{}]{RESET} Agent {YELLOW}{}{RESET}", i + 1, pane);
                println!("        {BOLD}Q:{RESET} {}", question);
                if !context.is_empty() {
                    println!("        {DIM}Context:{RESET} {}", context);
                }
            }
        }
    } else {
        println!("    {DIM}none{RESET}");
    }

    // ── WORKER HEALTH ─────────────────────────────────────────────────────────
    println!();
    println!("  {BOLD}{UNDERLINE}Workers{RESET}");

    let monitor_state = monitor::load_state();
    let panes: Vec<_> = tmux::list()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| is_worker_pane(p))
        .collect();

    if panes.is_empty() {
        println!("    {DIM}(no workers running){RESET}");
    } else {
        for p in &panes {
            let health = health::classify_pane(&p.id, &monitor_state, 60).ok();
            let status_colored = match &health {
                Some(h) => match h.status {
                    health::HealthStatus::Working => format!("{DIM}{GREEN}working{RESET} "),
                    health::HealthStatus::Idle => format!("{DIM}idle{RESET}    "),
                    health::HealthStatus::Stalled => format!("{BOLD}{RED}STALLED{RESET} "),
                    health::HealthStatus::Waiting => format!("{BOLD}{YELLOW}WAITING{RESET} "),
                    health::HealthStatus::Done => format!("{DIM}done{RESET}    "),
                },
                None => format!("{DIM}unknown{RESET} "),
            };
            let attn = match &health {
                Some(h) if h.needs_attention => {
                    format!("  {BOLD}{BRIGHT_RED}!! NEEDS ATTENTION{RESET}")
                }
                _ => String::new(),
            };
            let title = if p.title.is_empty() {
                &p.command
            } else {
                &p.title
            };
            let short_title: String = title.chars().take(48).collect();
            println!(
                "    {DIM}{}{RESET}  {status_colored}  {BOLD}{:<48}{RESET}{}",
                p.id, short_title, attn
            );
        }
    }

    println!();
    Ok(())
}

/// Handle `Command::Workers` — human-readable worker list display.
pub fn handle_workers() -> Result<()> {
    let panes: Vec<_> = tmux::list()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| is_worker_pane(p))
        .collect();

    // Abbreviate home directory in path
    let home = std::env::var("HOME").unwrap_or_default();
    let abbrev_path = |path: &str| -> String {
        if !home.is_empty() && path.starts_with(&home) {
            format!("~{}", &path[home.len()..])
        } else {
            path.to_string()
        }
    };

    // Hint bar
    println!("  {DIM}any key to close{RESET}");
    println!("  {DIM}{}{RESET}", "─".repeat(70));

    println!();
    if panes.is_empty() {
        println!("  {BOLD}Active Workers:{RESET} none");
        println!();
        println!("  {DIM}No workers currently running.{RESET}");
        println!(
            "  {DIM}Spawn one with:{RESET} superharness spawn --task \"...\" --dir /path --model <model>"
        );
    } else {
        // Column widths: PANE 6, CMD 10, STATUS 8, TITLE 40, PATH 30
        const W_PANE: usize = 6;
        const W_CMD: usize = 10;
        const W_TITLE: usize = 40;
        const W_PATH: usize = 30;
        // total separator width
        let sep_width = W_PANE + 2 + W_CMD + 2 + W_TITLE + 2 + W_PATH;

        println!("  {BOLD}Active Workers:{RESET} {}", panes.len());
        println!();
        println!(
            "  {BOLD}{UNDERLINE}{:<W_PANE$}  {:<W_CMD$}  {:<W_TITLE$}  {:<W_PATH$}{RESET}",
            "AGENT", "CMD", "TITLE", "PATH"
        );
        println!("  {DIM}{}{RESET}", "─".repeat(sep_width));
        for p in &panes {
            let title = if p.title.is_empty() {
                &p.command
            } else {
                &p.title
            };
            let short_title: String = title.chars().take(W_TITLE).collect();
            let path_abbrev = abbrev_path(&p.path);
            let short_path: String = path_abbrev.chars().take(W_PATH).collect();
            let short_cmd: String = p.command.chars().take(W_CMD).collect();
            println!(
                "  {DIM}{:<W_PANE$}{RESET}  {CYAN}{:<W_CMD$}{RESET}  {BOLD}{:<W_TITLE$}{RESET}  {DIM}{:<W_PATH$}{RESET}",
                p.id, short_cmd, short_title, short_path
            );
        }
    }
    println!();
    Ok(())
}

/// Handle `Command::StatusCounts` — brief active/total worker count for status bar.
pub fn handle_status_counts() -> Result<()> {
    println!("{}", heartbeat::status_counts());
    Ok(())
}

/// Format the F5 task-count label from (completed, total).
///
/// - `total == 0` (no tasks) → `"0"` — a bare zero, per the spec.
/// - otherwise → `"<completed>/<total>"`, e.g. `"2/4"`, `"0/4"`, `"1/666"`.
///
/// "Completed" is the count of tasks with status `done`; "total" is every task
/// in `tasks.json` regardless of status. Pure for testability.
pub fn format_task_counts(completed: usize, total: usize) -> String {
    if total == 0 {
        "0".to_string()
    } else {
        format!("{completed}/{total}")
    }
}

/// Read `.superharness/tasks.json` and return `(completed, total)` where
/// `completed` is the number of `done` tasks and `total` is the task count.
///
/// Accepts both supported shapes — the canonical `{ "tasks": [ ... ] }` wrapper
/// and a legacy bare `[ ... ]` array (matching `handle_tasks_modal`). Any read
/// or parse failure yields `(0, 0)` so the status bar degrades to `"0"` rather
/// than erroring.
fn read_task_counts() -> (usize, usize) {
    #[derive(serde::Deserialize)]
    struct OrchestratorTask {
        status: String,
    }
    #[derive(serde::Deserialize, Default)]
    struct TasksFile {
        #[serde(default)]
        tasks: Vec<OrchestratorTask>,
    }

    let tasks_path = match project::get_project_state_dir() {
        Ok(dir) => dir.join("tasks.json"),
        Err(_) => return (0, 0),
    };
    if !tasks_path.exists() {
        return (0, 0);
    }
    let content = std::fs::read_to_string(&tasks_path).unwrap_or_default();
    let tasks: Vec<OrchestratorTask> = serde_json::from_str::<TasksFile>(&content)
        .map(|f| f.tasks)
        .or_else(|_| serde_json::from_str::<Vec<OrchestratorTask>>(&content))
        .unwrap_or_default();

    // Normalize status so a worker writing "completed" (or other synonyms) is
    // still counted as done — see crate::tasks::TaskStatus.
    let total = tasks.len();
    let completed = tasks
        .iter()
        .filter(|t| crate::tasks::TaskStatus::from_raw(&t.status).is_completed())
        .count();
    (completed, total)
}

/// Handle `Command::TaskCounts` — completed/total task count for the F5 status
/// bar label (`tasks(2/4)`, or `tasks(0)` when there are no tasks).
pub fn handle_task_counts() -> Result<()> {
    let (completed, total) = read_task_counts();
    println!("{}", format_task_counts(completed, total));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::format_task_counts;

    #[test]
    fn task_counts_no_tasks_is_bare_zero() {
        // Both zero → a single "0" in the (brackets added by the status bar).
        assert_eq!(format_task_counts(0, 0), "0");
    }

    #[test]
    fn task_counts_some_done() {
        assert_eq!(format_task_counts(2, 4), "2/4");
        assert_eq!(format_task_counts(1, 666), "1/666");
    }

    #[test]
    fn task_counts_none_done_but_tasks_exist() {
        // Tasks exist but none done → "0/N", NOT the bare-zero form.
        assert_eq!(format_task_counts(0, 4), "0/4");
    }

    #[test]
    fn task_counts_all_done() {
        assert_eq!(format_task_counts(4, 4), "4/4");
    }
}

/// Handle `Command::TerminalSize` — terminal dimensions and layout recommendation.
pub fn handle_terminal_size() -> Result<()> {
    let info = tmux::terminal_size_info();
    let out = serde_json::json!({
        "width": info.width,
        "height": info.height,
        "main_pane_rows": info.main_pane_rows,
        "workers_visible": info.workers_visible,
        "recommended_max_workers": info.recommended_max_workers,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::ToggleMode` — toggle between away and present mode.
pub fn handle_toggle_mode() -> Result<()> {
    // Read project state to determine current mode
    let state_dir = project::get_project_state_dir()?;
    let state_file = state_dir.join("state.json");

    let current_mode = if state_file.exists() {
        let content = std::fs::read_to_string(&state_file).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        v["mode"].as_str().unwrap_or("present").to_string()
    } else {
        "present".to_string()
    };

    // Send the mode-switch message straight to the orchestrator pane. We must
    // NOT look it up via tmux::list(): that returns only `@sh_worker`-tagged
    // worker panes, so the orchestrator is never in it — the old lookup either
    // mis-targeted a worker or fell back to a bogus `%0` (which fails with
    // "can't find pane: %0" under base-index / embedded sessions).
    let target_pane = tmux::orchestrator_pane_id();

    let (message, new_mode) = if current_mode == "away" {
        (
            "The user has returned. Please read .superharness/state.json and .superharness/decisions.json to understand what happened while they were away, give them a brief natural-language debrief, then update .superharness/state.json to set mode to 'present' and clear away_since.",
            "present",
        )
    } else {
        (
            "The user wants to step away. Please ask them a few questions about what decisions you should queue vs auto-approve while they are gone, then update .superharness/state.json with {\"mode\": \"away\", \"away_since\": <unix_timestamp>, \"instructions\": <their preferences>} and adjust your behavior accordingly.",
            "away",
        )
    };

    tmux::send(&target_pane, message)?;

    let out = serde_json::json!({
        "toggled": true,
        "previous_mode": current_mode,
        "requesting_mode": new_mode,
        "target_pane": target_pane,
        "note": "Message sent to orchestrator. The AI will handle the mode transition."
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
