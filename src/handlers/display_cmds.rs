use crate::util::{BOLD, CYAN, DIM, GREEN, RED, RESET, UNDERLINE, YELLOW};
use crate::{events, project};
use anyhow::Result;

/// Handle `Command::EventFeed`.
pub fn handle_event_feed() -> Result<()> {
    let state_dir = project::get_project_state_dir()?;
    let events_path = state_dir.join("events.json");

    let all_events = events::load_events().unwrap_or_default();
    // Show last 200 events in chronological order (oldest first)
    let start = all_events.len().saturating_sub(200);
    let ev_slice = &all_events[start..];

    // Hint bar (first thing shown; q closes less, arrows scroll)
    println!("  {DIM}q:close  ↑/↓ or PgUp/PgDn:scroll  /:search{RESET}");
    println!("  {DIM}{}{RESET}", "─".repeat(70));

    println!();
    println!(
        "  {BOLD}Event Log:{RESET} {}  {DIM}({} total, showing last {}){RESET}",
        events_path.display(),
        all_events.len(),
        ev_slice.len()
    );
    println!();

    if ev_slice.is_empty() {
        println!("  {DIM}No events recorded yet.{RESET}");
    } else {
        for ev in ev_slice {
            let secs = ev.timestamp;
            let h = (secs % 86400) / 3600;
            let m = (secs % 3600) / 60;
            let s = secs % 60;
            let time_str = format!("{h:02}:{m:02}:{s:02}");

            let (color, kind_str) = match &ev.kind {
                events::EventKind::WorkerSpawned => (GREEN, format!("{}", ev.kind)),
                events::EventKind::WorkerKilled => (RED, format!("{}", ev.kind)),
                events::EventKind::WorkerCompleted => (CYAN, format!("{}", ev.kind)),
                events::EventKind::Pulse => (DIM, format!("{}", ev.kind)),
                _ => (YELLOW, format!("{}", ev.kind)),
            };

            let pane_str = ev
                .pane
                .as_deref()
                .map(|p| format!("  {DIM}{p}{RESET}"))
                .unwrap_or_default();

            let details = &ev.details;

            println!(
                "  {DIM}[{time_str}]{RESET}  {color}{kind_str:<20}{RESET}{pane_str}  {}",
                details.lines().next().unwrap_or("")
            );
            for cont_line in details.lines().skip(1) {
                println!("    {DIM}{cont_line}{RESET}");
            }
        }
    }
    println!();
    Ok(())
}

/// Handle `Command::TasksModal` — orchestrator tasks from .superharness/tasks.json.
pub fn handle_tasks_modal() -> Result<()> {
    #[derive(serde::Deserialize)]
    struct OrchestratorTask {
        id: String,
        title: String,
        #[serde(default)]
        description: String,
        status: String,
        #[serde(default)]
        priority: String,
        #[serde(default)]
        worker_pane: Option<String>,
    }

    let state_dir = project::get_project_state_dir()?;
    let tasks_path = state_dir.join("tasks.json");

    let task_list: Vec<OrchestratorTask> = if tasks_path.exists() {
        let content = std::fs::read_to_string(&tasks_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Count per status
    let count_in_progress = task_list
        .iter()
        .filter(|t| t.status == "in-progress")
        .count();
    let count_pending = task_list.iter().filter(|t| t.status == "pending").count();
    let count_blocked = task_list.iter().filter(|t| t.status == "blocked").count();
    let count_done = task_list.iter().filter(|t| t.status == "done").count();
    let count_cancelled = task_list.iter().filter(|t| t.status == "cancelled").count();

    // Hint bar
    println!("  {DIM}q:close  ↑/↓ or PgUp/PgDn:scroll  /:search{RESET}");
    println!("  {DIM}{}{RESET}", "─".repeat(70));

    println!();
    println!(
        "  {BOLD}Tasks:{RESET} {}  {DIM}| in-progress:{} pending:{} blocked:{} done:{} cancelled:{}{RESET}",
        task_list.len(),
        count_in_progress,
        count_pending,
        count_blocked,
        count_done,
        count_cancelled,
    );
    println!("  {DIM}{}{RESET}", "─".repeat(72));
    println!();

    if task_list.is_empty() {
        println!("  {DIM}No tasks found in {}{RESET}", tasks_path.display());
        println!();
    } else {
        // Order: in-progress, pending, blocked, done, cancelled
        let status_order = ["in-progress", "pending", "blocked", "done", "cancelled"];

        for status_key in &status_order {
            let group: Vec<&OrchestratorTask> = task_list
                .iter()
                .filter(|t| t.status == *status_key)
                .collect();
            if group.is_empty() {
                continue;
            }

            let (color, label) = match *status_key {
                "in-progress" => (GREEN, "IN-PROGRESS"),
                "pending" => (YELLOW, "PENDING"),
                "blocked" => (RED, "BLOCKED"),
                "done" => (DIM, "DONE"),
                "cancelled" => (DIM, "CANCELLED"),
                _ => ("\x1b[0m", *status_key),
            };

            println!("  {BOLD}{UNDERLINE}{color}{label}{RESET}");
            println!();

            for task in &group {
                let priority_badge = match task.priority.as_str() {
                    "high" => format!("{RED}[HIGH]{RESET} "),
                    "medium" => format!("{YELLOW}[MED]{RESET}  "),
                    "low" => format!("{DIM}[LOW]{RESET}  "),
                    _ => String::new(),
                };

                let desc_preview: String = task.description.chars().take(80).collect();
                let desc_suffix = if task.description.len() > 80 {
                    "…"
                } else {
                    ""
                };

                let pane_str = task
                    .worker_pane
                    .as_deref()
                    .map(|p| format!("  {DIM}pane:{p}{RESET}"))
                    .unwrap_or_default();

                println!(
                    "  {color}[{label}]{RESET} {priority_badge}{BOLD}{}{RESET}{pane_str}",
                    task.title
                );
                if !desc_preview.is_empty() {
                    println!("    {DIM}{}{}{RESET}", desc_preview, desc_suffix);
                }
                println!("    {DIM}id: {}{RESET}", task.id);
                println!();
            }
        }
    }

    Ok(())
}
