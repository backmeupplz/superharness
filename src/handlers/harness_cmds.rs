use crate::{harness, tmux, util};
use anyhow::Result;
use std::io::{self, Write};

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
    harness::validate_harness_name(&name)?;
    harness::set_default_harness(&config_dir, &name)?;
    println!("Default harness set to: {name}");
    Ok(())
}

/// Handle `Command::HarnessSwitch`.
pub fn handle_harness_switch(name: String) -> Result<()> {
    // Refuse to switch if any worker panes are running
    let orch_id = tmux::orchestrator_pane_id();
    let panes = tmux::list().unwrap_or_default();
    let worker_panes: Vec<_> = panes.iter().filter(|p| p.id != orch_id).collect();
    if !worker_panes.is_empty() {
        let ids: Vec<&str> = worker_panes.iter().map(|p| p.id.as_str()).collect();
        anyhow::bail!(
            "Cannot switch harness while workers are running: {}.\n\
             Kill all workers first with 'superharness kill --pane <id>', then retry.",
            ids.join(", ")
        );
    }

    harness::validate_harness_name(&name)?;

    let config_dir = util::superharness_config_dir();
    harness::set_default_harness(&config_dir, &name)?;
    println!("Harness switched to: {name}");
    println!("Workers spawned from now on will use '{name}'.");
    Ok(())
}

/// Handle `Command::HarnessSettings` — interactive harness picker.
pub fn handle_harness_settings() -> Result<()> {
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
