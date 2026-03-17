use anyhow::Result;

use crate::{events, pending_tasks, tmux};

/// Perform the git worktree status warning check for a directory.
/// Shared between `handle_spawn` and `handle_git_check`.
///
/// Emits warnings on stderr when:
///   - the repo is in detached HEAD state, or
///   - there are uncommitted (dirty) files.
///
/// Returns `true` if the directory is a git repo (regardless of dirtiness).
pub fn check_worktree_status(dir_str: &str) -> bool {
    let is_git = std::process::Command::new("git")
        .args(["-C", dir_str, "rev-parse", "--git-dir"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !is_git {
        return false;
    }

    // ── Detached HEAD check ──────────────────────────────────────────────────
    let is_detached = std::process::Command::new("git")
        .args(["-C", dir_str, "symbolic-ref", "--quiet", "HEAD"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

    if is_detached {
        eprintln!("WARNING: {dir_str} is in detached HEAD state.");
        eprintln!("  A worktree created from here will not be on any branch.");
        eprintln!("  Consider checking out a branch first:");
        eprintln!("    git -C {dir_str} checkout -b <branch-name>");
    }

    // ── Dirty-files check ────────────────────────────────────────────────────
    if let Ok(out) = std::process::Command::new("git")
        .args(["-C", dir_str, "status", "--porcelain"])
        .output()
    {
        let status_text = String::from_utf8_lossy(&out.stdout);
        let dirty_lines: Vec<&str> = status_text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let dirty_count = dirty_lines.len();
        if dirty_count > 0 {
            let staged = dirty_lines
                .iter()
                .filter(|l| {
                    let b = l.as_bytes();
                    !b.is_empty() && b[0] != b' ' && b[0] != b'?'
                })
                .count();
            let unstaged = dirty_lines
                .iter()
                .filter(|l| {
                    let b = l.as_bytes();
                    b.len() > 1 && b[1] != b' ' && b[0] == b' '
                })
                .count();
            let untracked = dirty_lines.iter().filter(|l| l.starts_with("??")).count();

            eprintln!(
                "WARNING: {dir_str} has {dirty_count} file(s) with uncommitted changes \
                 ({staged} staged, {unstaged} unstaged, {untracked} untracked)."
            );
            eprintln!("  If you are using a git worktree, dirty files will NOT be included.");
            eprintln!("  Commit or stash them first, or run for details:");
            eprintln!("    superharness git-check --dir {dir_str}");
        }
    }

    true
}

/// Handle `Command::Spawn`.
pub fn handle_spawn(
    task: String,
    dir: String,
    name: Option<String>,
    model: Option<String>,
    harness: Option<String>,
    mode: Option<String>,
    depends_on: Option<String>,
    no_hide: bool,
) -> Result<()> {
    if std::env::var("SUPERHARNESS_WORKER").is_ok() {
        eprintln!("error: workers cannot spawn sub-workers (SUPERHARNESS_WORKER is set)");
        std::process::exit(1);
    }

    if let Some(ref m) = mode {
        match m.as_str() {
            "build" | "plan" => {}
            other => anyhow::bail!(
                "invalid mode {:?}: must be 'build' (default) or 'plan' (read-only planning)",
                other
            ),
        }
    }

    // Warn if the target dir is a git repo with uncommitted changes or is in
    // a state that can make worktrees tricky (detached HEAD, no commits).
    {
        let check_dir =
            std::fs::canonicalize(&dir).unwrap_or_else(|_| std::path::PathBuf::from(&dir));
        let check_dir_str = check_dir.to_string_lossy().to_string();
        check_worktree_status(&check_dir_str);
    }

    // If --depends-on is provided, defer execution until dependencies finish.
    if let Some(deps_str) = depends_on {
        let deps: Vec<String> = deps_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let id = pending_tasks::add_task(pending_tasks::PendingTaskInput {
            task: task.clone(),
            dir,
            model,
            mode,
            name,
            harness,
            depends_on: deps.clone(),
        })?;
        let out = serde_json::json!({
            "pending": true,
            "task_id": id,
            "depends_on": deps,
            "note": "Task queued. Run 'run-pending' to spawn it once dependencies finish."
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        let pane = tmux::spawn(
            &task,
            &dir,
            name.as_deref(),
            model.as_deref(),
            harness.as_deref(),
            mode.as_deref(),
            no_hide,
        )?;
        let short_task: String = task.chars().take(80).collect();
        let _ = events::log_event(events::EventKind::WorkerSpawned, Some(&pane), &short_task);

        let out = serde_json::json!({ "pane": pane });
        println!("{}", serde_json::to_string_pretty(&out)?);
    }

    Ok(())
}
