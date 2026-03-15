use anyhow::Result;

use crate::{harness, project, setup, tasks, tmux, util};

/// Handle the default (no subcommand) case: first-launch harness picker,
/// write config, and initialise the tmux session.
pub fn handle_init(dir: &str, bin: &str) -> Result<()> {
    // Record active project directory
    let abs_dir = std::fs::canonicalize(dir).unwrap_or_else(|_| std::path::PathBuf::from(dir));
    project::set_active_project(&abs_dir)?;

    // Silently prune done/cancelled tasks so the orchestrator never sees stale entries.
    tasks::cleanup_completed_tasks();

    // First-launch harness picker — if no default harness is configured,
    // show an interactive picker before the tmux session starts.
    {
        let config_dir = util::superharness_config_dir();
        if harness::get_default_harness(&config_dir).is_none() {
            let candidates = harness::detect_all_candidates();
            if !candidates.is_empty() {
                println!("Welcome to SuperHarness! Please select your default AI harness:");
                println!();
                match harness::run_interactive_picker(&candidates, None) {
                    Ok(Some(chosen)) => {
                        let _ = harness::set_default_harness(&config_dir, &chosen);
                        println!("  Default harness set to: {chosen}");
                    }
                    _ => {}
                }
                println!();
            }
        }
    }

    setup::write_config(dir, bin)?;
    tmux::init(dir, bin)?;
    Ok(())
}
