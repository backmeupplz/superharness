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
        Ok(Some(chosen)) => {
            // Check if harness actually changed
            let harness_changed = current_harness.as_deref() != Some(&chosen);

            match harness::set_default_harness(&config_dir, &chosen) {
                Ok(()) => {
                    println!(
                        "  \x1b[1;32m\u{2713}\x1b[0m Default harness set to: \x1b[1m{chosen}\x1b[0m"
                    );

                    // Restart orchestrator if harness changed and we're in a superharness session
                    if harness_changed {
                        if let Err(e) = restart_orchestrator_with_new_harness(&chosen) {
                            eprintln!("  \x1b[1;33m!\x1b[0m Could not restart orchestrator: {e}");
                            eprintln!("  New harness will take effect for future workers.");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  error: could not save config: {e}");
                }
            }
        }
        Ok(None) => {
            println!("  No changes made.");
        }
        Err(e) => {
            eprintln!("  picker error: {e}");
        }
    }
    Ok(())
}

/// Restart the orchestrator pane (%0) with the new harness.
/// This kills the current orchestrator and respawns it with the new harness.
fn restart_orchestrator_with_new_harness(new_harness: &str) -> Result<()> {
    // Check if we're inside a tmux session
    let in_tmux = std::env::var("TMUX")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if !in_tmux {
        // Not in tmux, nothing to restart
        return Ok(());
    }

    // Check if superharness session exists
    let has_session = std::process::Command::new("tmux")
        .args(["has-session", "-t", tmux::SESSION])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !has_session {
        // No superharness session running, nothing to restart
        return Ok(());
    }

    let orch_pane = tmux::orchestrator_pane_id();

    // Build the new harness command with a resume prompt
    let config_dir = util::superharness_config_dir();
    let default_model = harness::get_default_model(&config_dir);

    // Create a resume prompt that tells the user the harness was switched
    let resume_prompt = format!(
        "[SUPERHARNESS] Harness switched to: {}. The orchestrator has been restarted with the new harness. \
         Please acknowledge and continue from where you left off, or ask what you'd like to work on.",
        new_harness
    );

    let harness_cmd =
        harness::build_harness_cmd(new_harness, default_model.as_deref(), &resume_prompt);

    // Respawn the orchestrator pane with the new harness
    // Use -k to kill any existing process in the pane
    let splash = format!(
        "printf '\033[2J\033[H\033[?25l\033[38;5;214mRestarting orchestrator with {new_harness}...\033[0m'; exec {harness_cmd}"
    );

    tmux::tmux_ok(&[
        "respawn-pane",
        "-t",
        &orch_pane,
        "-k",
        "bash",
        "-lc",
        &splash,
    ])?;

    println!("  \x1b[1;32m\u{2713}\x1b[0m Orchestrator restarted with \x1b[1m{new_harness}\x1b[0m");

    Ok(())
}
