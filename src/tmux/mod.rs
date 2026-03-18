use anyhow::{bail, Context, Result};
use std::process::Command;

mod layout;
mod panes;
mod session;
mod terminal;

pub use layout::*;
pub use panes::*;
pub use session::*;
pub use terminal::*;

pub(crate) const SESSION: &str = "superharness";

/// The tmux environment variable where we store the orchestrator pane ID.
const ORCH_PANE_ENV: &str = "SUPERHARNESS_ORCH_PANE";

/// Return the orchestrator pane ID for the current superharness session.
///
/// Inside a fresh tmux server the orchestrator is always `%0`, but when
/// superharness is launched inside an existing tmux session the first pane
/// may receive a different globally-unique ID (e.g. `%5`).  We store the
/// actual pane ID as a tmux environment variable at session creation time
/// and read it back here.
///
/// Falls back to `%0` when the env var is not set (backward compatibility
/// with sessions created before this change).
pub fn orchestrator_pane_id() -> String {
    // Try reading from the tmux session environment first.
    if let Ok(output) = Command::new("tmux")
        .args(["show-environment", "-t", SESSION, ORCH_PANE_ENV])
        .output()
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            // tmux show-environment outputs: VARNAME=value
            if let Some(val) = raw.trim().strip_prefix(&format!("{ORCH_PANE_ENV}=")) {
                let id = val.trim().to_string();
                if !id.is_empty() {
                    return id;
                }
            }
        }
    }
    // Fallback for sessions created before the env var existed.
    "%0".to_string()
}

/// Store the orchestrator pane ID in the tmux session environment.
pub(crate) fn set_orchestrator_pane_id(pane_id: &str) -> Result<()> {
    tmux_ok(&["set-environment", "-t", SESSION, ORCH_PANE_ENV, pane_id])
}

/// Run a tmux command, return stdout
fn tmux(args: &[&str]) -> Result<String> {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .with_context(|| format!("failed to run: tmux {}", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "tmux {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn tmux_ok(args: &[&str]) -> Result<()> {
    tmux(args)?;
    Ok(())
}
