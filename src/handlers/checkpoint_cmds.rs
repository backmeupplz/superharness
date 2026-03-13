use crate::{checkpoint, memory, tmux};
use anyhow::Result;

/// Handle `Command::Checkpoint`.
pub fn handle_checkpoint(pane: String, note: Option<String>) -> Result<()> {
    // Capture current pane output (last 200 lines)
    let pane_output = tmux::read(&pane, 200)?;

    // Use the pane title as the task title; fall back to pane ID
    let pane_list = tmux::list()?;
    let task_title = pane_list
        .iter()
        .find(|p| p.id == pane)
        .map(|p| p.title.clone())
        .unwrap_or_else(|| pane.clone());

    let cp = checkpoint::save(&pane, &task_title, &pane_output, note.as_deref())?;
    let out = serde_json::json!({
        "checkpoint_id": cp.id,
        "pane": cp.pane_id,
        "timestamp": cp.timestamp,
        "task_title": cp.task_title,
        "note": cp.note,
        "lines_captured": cp.last_output.lines().count(),
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Checkpoints`.
pub fn handle_checkpoints(pane: Option<String>) -> Result<()> {
    let list = checkpoint::list(pane.as_deref())?;
    let out = serde_json::json!({ "checkpoints": list });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Resume`.
pub fn handle_resume(checkpoint_id: String, dir: String, model: Option<String>) -> Result<()> {
    let cp = checkpoint::load_by_id(&checkpoint_id)?;

    // Build a resume prompt containing context from the checkpoint
    let last_lines: String = cp
        .last_output
        .lines()
        .rev()
        .take(30)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    let resume_prompt = format!(
        "Resume this task. Previous context: {task_title}. \
        Last output was:\n{last_lines}\n\nContinue where it left off.",
        task_title = cp.task_title,
        last_lines = last_lines,
    );

    let note_suffix = cp
        .note
        .as_deref()
        .map(|n| format!(" (note: {n})"))
        .unwrap_or_default();
    let name = format!("resume of {}{}", cp.task_title, note_suffix);

    let pane_id = tmux::spawn(
        &resume_prompt,
        &dir,
        Some(&name),
        model.as_deref(),
        None, // use default harness for resumed worker
        Some("build"),
        false, // show in main window (default)
    )?;
    let out = serde_json::json!({
        "pane": pane_id,
        "resumed_from": checkpoint_id,
        "task_title": cp.task_title,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Memory`.
pub fn handle_memory(
    pane: String,
    key: Option<String>,
    value: Option<String>,
    list: bool,
) -> Result<()> {
    if list {
        let mem = memory::get_all(&pane)?;
        let out = serde_json::json!({
            "pane": mem.pane_id,
            "memory": mem.entries,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        match (key, value) {
            (Some(k), Some(v)) => {
                memory::set(&pane, &k, &v)?;
                let out = serde_json::json!({
                    "pane": pane,
                    "stored": true,
                    "key": k,
                    "value": v,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
            _ => {
                anyhow::bail!(
                    "provide --key and --value to store a fact, or --list to retrieve all"
                );
            }
        }
    }
    Ok(())
}
