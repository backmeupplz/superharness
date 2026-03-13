use anyhow::Result;

use crate::util::{BOLD, CYAN, DIM, GREEN, RED, RESET, UNDERLINE, YELLOW};
use crate::{
    checkpoint, events, harness, health, loop_guard, memory, pending_tasks, project, tmux, util,
};

// ── harness ─────────────────────────────────────────────────────────────────

/// Handle `Command::HarnessList`.
pub fn handle_harness_list() -> Result<()> {
    let config_dir = util::superharness_config_dir();

    let installed = harness::detect_installed();
    let default_name = harness::get_default_harness(&config_dir);

    if installed.is_empty() {
        println!("No AI harnesses detected on PATH.");
        println!();
        println!("Install one of the following:");
        println!("  opencode  (OpenCode)    — https://opencode.ai");
        println!("  claude    (Claude Code)  — https://claude.ai/code");
        println!("  codex     (OpenAI Codex) — https://github.com/openai/codex");
    } else {
        println!("Detected harnesses:");
        println!();
        for h in &installed {
            let is_default = default_name.as_deref() == Some(h.name.as_str())
                || (default_name.is_none()
                    && installed.first().map(|f| f.name == h.name).unwrap_or(false));
            let marker = if is_default { " *  (default)" } else { "" };
            println!("  {:<10}  {}{}", h.binary, h.display_name, marker);
        }
        println!();
        if let Some(ref d) = default_name {
            println!("Default (from config): {d}");
        } else {
            println!("Default (auto-selected): {}", installed[0].binary);
            println!("Set an explicit default with: superharness harness-set <name>");
        }
    }
    Ok(())
}

/// Handle `Command::HarnessSet`.
pub fn handle_harness_set(name: String) -> Result<()> {
    let config_dir = util::superharness_config_dir();

    // Validate: must be a known harness name
    let known = ["opencode", "claude", "codex"];
    if !known.contains(&name.as_str()) {
        anyhow::bail!(
            "Unknown harness {:?}. Valid options: opencode, claude, codex",
            name
        );
    }

    // Warn if the chosen harness is not actually installed
    let installed = harness::detect_installed();
    if !installed.iter().any(|h| h.name == name || h.binary == name) {
        eprintln!(
            "WARNING: '{name}' does not appear to be installed on PATH.\n\
             Install it from: {}",
            harness::install_url(&name)
        );
    }

    harness::set_default_harness(&config_dir, &name)?;
    println!("Default harness set to: {name}");
    Ok(())
}

/// Handle `Command::HarnessSwitch`.
pub fn handle_harness_switch(name: String) -> Result<()> {
    // Refuse to switch if any worker panes are running
    let panes = tmux::list().unwrap_or_default();
    let worker_panes: Vec<_> = panes.iter().filter(|p| p.id != "%0").collect();
    if !worker_panes.is_empty() {
        let ids: Vec<&str> = worker_panes.iter().map(|p| p.id.as_str()).collect();
        anyhow::bail!(
            "Cannot switch harness while workers are running: {}.\n\
             Kill all workers first with 'superharness kill --pane <id>', then retry.",
            ids.join(", ")
        );
    }

    // Validate name
    let known = ["opencode", "claude", "codex"];
    if !known.contains(&name.as_str()) {
        anyhow::bail!(
            "Unknown harness {:?}. Valid options: opencode, claude, codex",
            name
        );
    }

    // Warn if not installed
    let installed = harness::detect_installed();
    if !installed.iter().any(|h| h.name == name || h.binary == name) {
        eprintln!(
            "WARNING: '{name}' does not appear to be installed on PATH.\n\
             Install it from: {}",
            harness::install_url(&name)
        );
    }

    let config_dir = util::superharness_config_dir();
    harness::set_default_harness(&config_dir, &name)?;
    println!("Harness switched to: {name}");
    println!("Workers spawned from now on will use '{name}'.");
    Ok(())
}

/// Handle `Command::HarnessSettings` — interactive harness picker.
pub fn handle_harness_settings() -> Result<()> {
    use std::io::{self, Write};

    let config_dir = util::superharness_config_dir();

    let current_harness = harness::get_default_harness(&config_dir);
    let current_model = harness::get_default_model(&config_dir);

    // ── Show current settings ────────────────────────────────────────────────
    println!();
    println!("  \x1b[1mSuperHarness Settings\x1b[0m");
    println!("  {}", "─".repeat(50));
    println!();
    let harness_display = current_harness.as_deref().unwrap_or("(auto-detected)");
    let model_display = current_model.as_deref().unwrap_or("(none set)");
    println!("  Current harness : \x1b[1;32m{harness_display}\x1b[0m");
    println!("  Current model   : \x1b[1;33m{model_display}\x1b[0m");
    println!();
    println!("  \x1b[2mChange harness (↑↓ move, Enter select, q cancel):\x1b[0m");
    println!();
    io::stdout().flush().ok();

    // Collect ALL candidates (installed or not) so user can see the full list.
    let candidates: Vec<harness::HarnessInfo> = harness::detect_all_candidates();

    match harness::run_interactive_picker(&candidates, current_harness.as_deref()) {
        Ok(Some(chosen)) => match harness::set_default_harness(&config_dir, &chosen) {
            Ok(()) => {
                println!(
                    "  \x1b[1;32m\u{2713}\x1b[0m Default harness set to: \x1b[1m{chosen}\x1b[0m"
                );
            }
            Err(e) => {
                eprintln!("  error: could not save config: {e}");
            }
        },
        Ok(None) => {
            println!("  No changes made.");
        }
        Err(e) => {
            eprintln!("  picker error: {e}");
        }
    }
    Ok(())
}

// ── pane management ──────────────────────────────────────────────────────────

/// Handle `Command::List`.
pub fn handle_list() -> Result<()> {
    let panes = tmux::list()?;
    let out = serde_json::json!({ "panes": panes });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Read`.
pub fn handle_read(pane: String, lines: u32) -> Result<()> {
    let output = tmux::read(&pane, lines)?;
    let out = serde_json::json!({ "pane": pane, "output": output });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Send`.
pub fn handle_send(pane: String, text: String) -> Result<()> {
    tmux::send(&pane, &text)?;
    let out = serde_json::json!({ "pane": pane, "sent": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Kill`.
pub fn handle_kill(pane: String) -> Result<()> {
    use crate::heartbeat;
    tmux::kill(&pane)?;
    let _ = events::log_event(
        events::EventKind::WorkerKilled,
        Some(&pane),
        "worker killed",
    );
    // Trigger a heartbeat so the orchestrator wakes up immediately.
    let _ = heartbeat::heartbeat();
    let out = serde_json::json!({ "pane": pane, "killed": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Hide`.
pub fn handle_hide(pane: String, name: Option<String>) -> Result<()> {
    tmux::hide(&pane, name.as_deref())?;
    let out = serde_json::json!({ "pane": pane, "hidden": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Show`.
pub fn handle_show(pane: String, split: String) -> Result<()> {
    tmux::show(&pane, &split)?;
    let out = serde_json::json!({ "pane": pane, "visible": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Surface`.
pub fn handle_surface(pane: String) -> Result<()> {
    tmux::surface(&pane)?;
    let out = serde_json::json!({ "pane": pane, "visible": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Compact`.
pub fn handle_compact() -> Result<()> {
    let (moved, remaining) = tmux::compact_panes()?;
    let note = if moved > 0 {
        format!("{moved} agent(s) moved to background tabs. {remaining} agent(s) remain visible.")
    } else {
        "No agents needed moving — all agents meet size thresholds.".to_string()
    };
    let out = serde_json::json!({
        "moved_to_background": moved,
        "still_visible": remaining,
        "note": note,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Resize`.
pub fn handle_resize(pane: String, direction: String, amount: u32) -> Result<()> {
    tmux::resize(&pane, &direction, amount)?;
    let out = serde_json::json!({ "pane": pane, "resized": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Layout`.
pub fn handle_layout(name: String) -> Result<()> {
    tmux::layout(&name)?;
    let out = serde_json::json!({ "layout": name });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::SmartLayout`.
pub fn handle_smart_layout(hint: Option<String>) -> Result<()> {
    let action = match hint.as_deref() {
        // "maximize <pane_id>" — give that pane extra space and surface it
        Some(h) if h.starts_with("maximize ") => {
            let pane_id = h["maximize ".len()..].trim();
            tmux::smart_layout_with_attention(Some(pane_id))?;
            format!("maximized {pane_id}")
        }
        // "focus <pane_id>" — surface then rebalance
        Some(h) if h.starts_with("focus ") => {
            let pane_id = h["focus ".len()..].trim();
            tmux::surface(pane_id)?;
            tmux::smart_layout()?;
            format!("focused {pane_id}")
        }
        // "rebalance" or no hint — standard smart layout
        _ => {
            tmux::smart_layout()?;
            "rebalanced".to_string()
        }
    };
    let out = serde_json::json!({ "layout": "smart", "action": action, "hint": hint });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

// ── diagnostics / loop guard ─────────────────────────────────────────────────

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

// ── pending tasks ────────────────────────────────────────────────────────────

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

// ── checkpoint / memory ──────────────────────────────────────────────────────

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

// ── event feed / tasks modal ─────────────────────────────────────────────────

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
