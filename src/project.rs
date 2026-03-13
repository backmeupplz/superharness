//! Project-local state helpers.
//!
//! Active project tracking:
//!   ~/.local/share/superharness/active_project.txt  — path to the current project dir
//!
//! Project-local state directory:
//!   {project_dir}/.superharness/  — all project-specific state files live here

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Path to the global file that records the active project directory.
fn active_project_file() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("cannot determine home directory (HOME not set)")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
        .join("active_project.txt"))
}

/// Write the given directory as the active project.
/// Called on startup (no subcommand) to record which project is active.
pub fn set_active_project(dir: &std::path::Path) -> Result<()> {
    let file = active_project_file()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create superharness data dir: {}",
                parent.display()
            )
        })?;
    }
    std::fs::write(&file, dir.to_string_lossy().as_bytes())
        .with_context(|| format!("failed to write active_project.txt: {}", file.display()))?;
    Ok(())
}

/// Return the active project directory.
/// Reads ~/.local/share/superharness/active_project.txt.
/// Falls back to the current working directory if the file is missing or empty.
pub fn get_project_dir() -> Result<PathBuf> {
    let file = active_project_file()?;
    if file.exists() {
        let content = std::fs::read_to_string(&file)
            .with_context(|| format!("failed to read active_project.txt: {}", file.display()))?;
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    // Fallback: current directory
    std::env::current_dir().context("failed to get current working directory")
}

/// Return the project-local state directory: `{project_dir}/.superharness/`.
/// Creates the directory if it does not exist.
pub fn get_project_state_dir() -> Result<PathBuf> {
    let project_dir = get_project_dir()?;
    let state_dir = project_dir.join(".superharness");
    std::fs::create_dir_all(&state_dir).with_context(|| {
        format!(
            "failed to create .superharness directory: {}",
            state_dir.display()
        )
    })?;
    Ok(state_dir)
}
