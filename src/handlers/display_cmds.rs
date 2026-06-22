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

    // The orchestrator writes tasks.json freeform, so accept both supported
    // top-level shapes: the canonical `{ "tasks": [ ... ] }` wrapper object and
    // a legacy bare `[ ... ]` array. Anything else parses to an empty list.
    #[derive(serde::Deserialize, Default)]
    struct TasksFile {
        #[serde(default)]
        tasks: Vec<OrchestratorTask>,
    }

    let state_dir = project::get_project_state_dir()?;
    let tasks_path = state_dir.join("tasks.json");

    let task_list: Vec<OrchestratorTask> = if tasks_path.exists() {
        let content = std::fs::read_to_string(&tasks_path).unwrap_or_default();
        serde_json::from_str::<TasksFile>(&content)
            .map(|f| f.tasks)
            .or_else(|_| serde_json::from_str::<Vec<OrchestratorTask>>(&content))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    use crate::tasks::TaskStatus;

    // Count per CANONICAL status, so synonyms like "completed" fold into Done
    // and unrecognized statuses are surfaced under "other:" instead of vanishing.
    let count = |st: TaskStatus| {
        task_list
            .iter()
            .filter(|t| TaskStatus::from_raw(&t.status) == st)
            .count()
    };

    // Hint bar
    println!("  {DIM}q:close  ↑/↓ or PgUp/PgDn:scroll  /:search{RESET}");
    println!("  {DIM}{}{RESET}", "─".repeat(70));

    println!();
    println!(
        "  {BOLD}Tasks:{RESET} {}  {DIM}| in-progress:{} pending:{} blocked:{} done:{} cancelled:{} other:{}{RESET}",
        task_list.len(),
        count(TaskStatus::InProgress),
        count(TaskStatus::Pending),
        count(TaskStatus::Blocked),
        count(TaskStatus::Done),
        count(TaskStatus::Cancelled),
        count(TaskStatus::Other),
    );
    println!("  {DIM}{}{RESET}", "─".repeat(72));
    println!();

    if task_list.is_empty() {
        println!("  {DIM}No tasks found in {}{RESET}", tasks_path.display());
        println!();
    } else {
        // Group by canonical status in display order. `Other` is included so any
        // unrecognized status is still shown (never silently dropped).
        for status in TaskStatus::DISPLAY_ORDER {
            let group: Vec<&OrchestratorTask> = task_list
                .iter()
                .filter(|t| TaskStatus::from_raw(&t.status) == status)
                .collect();
            if group.is_empty() {
                continue;
            }

            let color = match status {
                TaskStatus::InProgress => GREEN,
                TaskStatus::Pending => YELLOW,
                TaskStatus::Blocked => RED,
                TaskStatus::Done => DIM,
                TaskStatus::Cancelled => DIM,
                TaskStatus::Other => CYAN,
            };

            println!("  {BOLD}{UNDERLINE}{color}{}{RESET}", status.label());
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

                // For unrecognized statuses, show the RAW status string the
                // orchestrator wrote so the user sees exactly what it is.
                let badge = if status == TaskStatus::Other {
                    format!("{color}[{}]{RESET}", task.status)
                } else {
                    format!("{color}[{}]{RESET}", status.label())
                };

                println!(
                    "  {badge} {priority_badge}{BOLD}{}{RESET}{pane_str}",
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
