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
