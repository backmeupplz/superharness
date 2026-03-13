use crate::{health, loop_guard, tmux};
use anyhow::Result;

/// Handle `Command::Ask`.
pub fn handle_ask(pane: String) -> Result<()> {
    let output = tmux::read(&pane, 20)?;
    let lines: Vec<&str> = output.lines().collect();

    // Patterns that suggest the worker is asking something
    let question_patterns: &[&str] = &[
        "?",
        "y/n",
        "Y/N",
        "yes/no",
        "Yes/No",
        "[y/n]",
        "[Y/N]",
        "Do you want",
        "Would you like",
        "Should I",
        "Can I",
        "Please confirm",
        "Enter ",
        "Provide ",
        "What ",
        "Which ",
        "How ",
        "Allow",
        "Approve",
        "Permission",
        "confirm",
        "proceed",
        "(y)",
        "(n)",
    ];

    let mut question_lines: Vec<(usize, &str)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        let is_question = question_patterns
            .iter()
            .any(|p| line.contains(p) || lower.contains(&p.to_lowercase()));
        if is_question && !line.trim().is_empty() {
            question_lines.push((i, line));
        }
    }

    println!("=== Agent {} — last {} lines ===", pane, lines.len());
    println!();
    for line in &lines {
        println!("  {line}");
    }
    println!();

    if question_lines.is_empty() {
        println!("[ No question or permission prompt detected ]");
        println!();
        println!("Worker appears to be working. Check back in 30-60s.");
    } else {
        println!("[ QUESTION / PROMPT DETECTED ]");
        println!();
        for (_, line) in &question_lines {
            println!("  >> {line}");
        }
        println!();
        println!("To answer, run:");
        println!("  superharness send --pane {pane} --text \"<your answer>\"");
        println!();
        println!("To approve (yes):  superharness send --pane {pane} --text \"y\"");
        println!("To deny (no):      superharness send --pane {pane} --text \"n\"");
    }

    Ok(())
}

/// Handle `Command::Respawn`.
pub fn handle_respawn(
    pane: String,
    task: String,
    dir: String,
    model: Option<String>,
    mode: Option<String>,
) -> Result<()> {
    // 1. Read last 100 lines for crash context
    let crash_context = tmux::read(&pane, 100)?;

    // 2. Kill the crashed pane
    tmux::kill(&pane)?;

    // 3. Build the retry task with crash context prepended
    let retry_task = format!(
        "Previous attempt crashed. Context from crash:\n{crash_context}\n\nPlease retry the task, avoiding whatever caused the crash.\n\nOriginal task: {task}"
    );

    // 4. Spawn a new worker
    let new_pane = tmux::spawn(
        &retry_task,
        &dir,
        None,
        model.as_deref(),
        None, // use default harness for respawned worker
        mode.as_deref(),
        false, // show in main window (default)
    )?;

    println!("Crashed agent {} killed.", pane);
    println!("New worker spawned: {new_pane}");
    println!();
    println!("The new worker has been given the crash context and will retry the task.");
    println!("Monitor with: superharness read --pane {new_pane} --lines 50");
    Ok(())
}

/// Handle `Command::LoopStatus`.
pub fn handle_loop_status(pane: Option<String>) -> Result<()> {
    match pane {
        Some(pane_id) => {
            let detection = loop_guard::get_loop_status(&pane_id)?;
            let out = serde_json::json!({
                "pane": pane_id,
                "loop_detected": detection.as_ref().map(|d| d.detected).unwrap_or(false),
                "details": detection
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        None => {
            let all_panes = loop_guard::get_all_panes()?;
            let mut results = Vec::new();
            for (pane_id, _count) in &all_panes {
                let detection = loop_guard::get_loop_status(pane_id)?;
                results.push(serde_json::json!({
                    "pane": pane_id,
                    "loop_detected": detection.as_ref().map(|d| d.detected).unwrap_or(false),
                    "details": detection
                }));
            }
            let out = serde_json::json!({ "panes": results });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }
    Ok(())
}

/// Handle `Command::LoopClear`.
pub fn handle_loop_clear(pane: String) -> Result<()> {
    loop_guard::clear_pane(&pane)?;
    let out = serde_json::json!({ "pane": pane, "cleared": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Healthcheck`.
pub fn handle_healthcheck(pane: Option<String>, interval: u64) -> Result<()> {
    health::run(pane.as_deref(), interval)?;
    Ok(())
}
