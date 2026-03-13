use crate::{pending_tasks, tmux};
use anyhow::Result;

/// Handle `Command::Tasks` — list pending (dependency-gated) tasks.
pub fn handle_tasks() -> Result<()> {
    let task_list = pending_tasks::list_tasks()?;
    // Enrich each task with dependency status using current tmux pane list
    let active_panes: Vec<String> = tmux::list()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
        .collect();
    let enriched: Vec<serde_json::Value> = task_list
        .iter()
        .map(|t| {
            let deps_status: Vec<serde_json::Value> = t
                .depends_on
                .iter()
                .map(|dep| {
                    serde_json::json!({
                        "pane": dep,
                        "done": !active_panes.contains(dep)
                    })
                })
                .collect();
            let ready = deps_status
                .iter()
                .all(|d| d["done"].as_bool().unwrap_or(false));
            serde_json::json!({
                "id": t.id,
                "task": t.task,
                "dir": t.dir,
                "model": t.model,
                "mode": t.mode,
                "name": t.name,
                "depends_on": deps_status,
                "ready": ready,
                "created_at": t.created_at
            })
        })
        .collect();
    let out = serde_json::json!({ "pending_tasks": enriched });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::RunPending`.
pub fn handle_run_pending() -> Result<()> {
    let active_panes: Vec<String> = tmux::list()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
        .collect();
    let ready = pending_tasks::ready_tasks(&active_panes)?;
    let mut spawned = Vec::new();
    for t in ready {
        match tmux::spawn(
            &t.task,
            &t.dir,
            t.name.as_deref(),
            t.model.as_deref(),
            t.harness.as_deref(),
            t.mode.as_deref(),
            false, // show in main window (default)
        ) {
            Ok(pane_id) => {
                pending_tasks::remove_task(&t.id)?;
                spawned.push(serde_json::json!({
                    "task_id": t.id,
                    "pane": pane_id,
                    "task": t.task
                }));
            }
            Err(e) => {
                spawned.push(serde_json::json!({
                    "task_id": t.id,
                    "error": e.to_string(),
                    "task": t.task
                }));
            }
        }
    }
    let out = serde_json::json!({ "spawned": spawned });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
