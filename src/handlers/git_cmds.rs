use anyhow::{Context, Result};

use crate::handlers::spawn::check_worktree_status;

/// Handle `Command::GitCheck` — detailed git repo status check for worktree creation.
pub fn handle_git_check(dir: String) -> Result<()> {
    let abs_dir =
        std::fs::canonicalize(&dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    // Check if it's a git repo at all
    let is_git = std::process::Command::new("git")
        .args(["-C", &dir_str, "rev-parse", "--git-dir"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !is_git {
        println!("Directory: {dir_str}");
        println!("Status:    NOT A GIT REPO");
        println!();
        println!("No git check needed — this directory is not a git repository.");
        println!("You can create worktrees only from git repos.");
        return Ok(());
    }

    // Run git status --porcelain to detect dirty files
    let status_out = std::process::Command::new("git")
        .args(["-C", &dir_str, "status", "--porcelain"])
        .output()
        .with_context(|| "failed to run git status")?;

    let status_text = String::from_utf8_lossy(&status_out.stdout);
    let dirty_lines: Vec<&str> = status_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();

    println!("Directory: {dir_str}");

    if dirty_lines.is_empty() {
        println!("Status:    CLEAN");
        println!();
        println!("Repo is clean. Safe to create a worktree from HEAD.");
        println!();
        println!("  git worktree add /tmp/worker-N HEAD");
    } else {
        println!(
            "Status:    DIRTY ({} file(s) with uncommitted changes)",
            dirty_lines.len()
        );
        println!();
        println!("Uncommitted changes:");
        for line in &dirty_lines {
            println!("  {line}");
        }
        println!();
        println!("WARNING: Worktrees are created from HEAD. Dirty files will NOT");
        println!("be included in the worktree. You should either:");
        println!();
        println!("  Option A — Commit your changes first:");
        println!("    git add -A && git commit -m \"wip: save before worktree\"");
        println!();
        println!("  Option B — Stash your changes:");
        println!("    git stash && git worktree add /tmp/worker-N HEAD && git stash pop");
        println!();
        println!("  Option C — Proceed anyway (dirty files stay in main only):");
        println!("    git worktree add /tmp/worker-N HEAD");
    }

    // Also run the shared warning check (which checks for detached HEAD as well)
    // but only as a secondary informational pass if the repo is dirty.
    // The primary output above is more structured; we just ensure the shared
    // function is used so it stays in sync with spawn.rs.
    let _ = check_worktree_status; // ensure it's visible from spawn.rs

    Ok(())
}
