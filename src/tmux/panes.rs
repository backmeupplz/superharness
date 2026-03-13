use anyhow::{Context, Result};
use serde::Serialize;

use crate::harness;
use crate::loop_guard;
use crate::util;

use super::{tmux, tmux_ok, SESSION};

/// Subtle RGB background tints for pane backgrounds.
/// Each is a very dark colour with just enough hue to be faintly distinct (~5% tint on black).
const PANE_COLOR_HEX: &[&str] = &[
    "#0d1117", // near-black blue-grey (GitHub dark style)
    "#0f110d", // near-black green tint
    "#110d0d", // near-black red tint
    "#100d11", // near-black purple tint
    "#11100d", // near-black amber tint
    "#0d1011", // near-black teal tint
    "#110d0f", // near-black rose tint
    "#0f100d", // near-black olive tint
];

/// Spawn a new opencode worker as a pane in the superharness window.
///
/// By default the new pane is immediately hidden to a background tab so the
/// main orchestrator window stays clean and full-size. Pass `no_hide = true`
/// to keep the worker visible in the main window (useful when you want to
/// watch a worker directly without surfacing it manually).
///
/// `harness_override` — when `Some`, use that harness binary directly instead
/// of the configured default.  Pass `None` to use the default.
pub fn spawn(
    task: &str,
    dir: &str,
    name: Option<&str>,
    model: Option<&str>,
    harness_override: Option<&str>,
    mode: Option<&str>,
    no_hide: bool,
) -> Result<String> {
    super::session::ensure_session()?;

    // Give a friendly error when the working directory is missing.
    if !std::path::Path::new(dir).exists() {
        anyhow::bail!(
            "directory does not exist: {dir}\n\
             Create it first with:\n\
             \n\
               mkdir -p {dir}"
        );
    }
    let abs_dir = std::fs::canonicalize(dir)
        .with_context(|| format!("could not resolve directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    let effective_mode = mode.unwrap_or("build");

    // In plan mode, prefix the task to instruct the agent not to make changes.
    let effective_task = match effective_mode {
        "plan" => format!("[PLAN MODE - do not make changes, only analyze and plan]: {task}"),
        _ => task.to_string(),
    };

    // Resolve which AI harness to invoke (opencode / claude / codex / …).
    let config_dir = util::superharness_config_dir();
    let active_harness = match harness_override {
        Some(h) => h.to_string(),
        None => harness::resolve_harness(&config_dir)?,
    };

    // Build the harness command string (handles per-harness flag differences).
    let opencode_cmd = harness::build_harness_cmd(&active_harness, model, &effective_task);

    // We wrap opencode so that when it exits the pane auto-kills itself.
    // The wrapper uses tmux display-message to get the pane's own ID at runtime,
    // then invokes `superharness kill --pane <id>` after opencode finishes.
    // We resolve the superharness binary path at spawn time so the worker pane
    // can always find it even if PATH is different inside the new bash session.
    let sh_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "superharness".to_string());

    let cmd = format!(
        "export SUPERHARNESS_WORKER=1; {opencode_cmd} ; {sh_bin} kill --pane $(tmux display-message -p '#{{pane_id}}')"
    );

    // Split the current window to create a new pane running opencode directly
    let pane_id = tmux(&[
        "split-window",
        "-t",
        SESSION,
        "-d", // don't switch focus
        "-P", // print pane info
        "-F",
        "#{pane_id}",
        "-c",
        &dir_str,
        "bash",
        "-lc",
        &cmd,
    ])?;

    // Set pane title: use explicit name if provided, otherwise "[mode] first 50 chars of task"
    let title = match name {
        Some(n) if !n.is_empty() => format!("[{effective_mode}] {n}"),
        _ => {
            let short_task: String = task.chars().take(50).collect();
            format!("[{effective_mode}] {short_task}")
        }
    };
    let _ = tmux_ok(&["select-pane", "-t", &pane_id, "-T", &title]);

    // Apply a subtle background tint from the palette based on pane index
    let pane_index_str =
        tmux(&["display-message", "-t", &pane_id, "-p", "#{pane_index}"]).unwrap_or_default();
    let pane_index: usize = pane_index_str.trim().parse().unwrap_or(0);
    let color_hex = PANE_COLOR_HEX[pane_index % PANE_COLOR_HEX.len()];
    let style = format!("bg={color_hex}");
    let _ = tmux_ok(&["select-pane", "-t", &pane_id, "-P", &style]);

    if no_hide {
        // --no-hide: keep the worker visible in the main orchestrator window.
        // Apply smart_layout + auto_compact to keep the arrangement tidy.
        let _ = super::smart_layout();
        let _ = super::auto_compact();
    } else {
        // Default: immediately move the new pane to a background tab so the
        // orchestrator window stays clean and full-size.
        let label = match name {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => pane_id.clone(),
        };
        let _ = hide(&pane_id, Some(&label));
    }

    Ok(pane_id)
}

/// Read recent output from a pane.
pub fn read(pane: &str, lines: u32) -> Result<String> {
    tmux(&["capture-pane", "-t", pane, "-p", "-S", &format!("-{lines}")])
}

/// Send text to a pane.
pub fn send(pane: &str, text: &str) -> Result<()> {
    tmux_ok(&["send-keys", "-t", pane, text, "Enter"])?;
    // Record this send action for loop detection; don't fail if loop guard errors
    if let Err(e) = loop_guard::record_action(pane, "send", text) {
        eprintln!("loop_guard: failed to record action: {e}");
    }
    Ok(())
}

/// Flash a notification message in the tmux status bar for 6 seconds.
pub fn flash_notification(msg: &str) -> Result<()> {
    tmux_ok(&["display-message", "-t", SESSION, "-d", "6000", msg])
}

#[derive(Serialize)]
pub struct PaneInfo {
    pub id: String,
    pub window: String,
    pub command: String,
    pub path: String,
    pub title: String,
}

/// List all panes across all windows in the session.
pub fn list() -> Result<Vec<PaneInfo>> {
    if !super::session::has_session() {
        return Ok(vec![]);
    }

    let output = tmux(&[
        "list-panes",
        "-t",
        SESSION,
        "-a",
        "-F",
        "#{pane_id}\t#{window_name}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_title}",
    ])?;

    let panes = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(5, '\t').collect();
            PaneInfo {
                id: parts.first().unwrap_or(&"").to_string(),
                window: parts.get(1).unwrap_or(&"").to_string(),
                command: parts.get(2).unwrap_or(&"").to_string(),
                path: parts.get(3).unwrap_or(&"").to_string(),
                title: parts.get(4).unwrap_or(&"").to_string(),
            }
        })
        .collect();

    Ok(panes)
}

/// Attempt to remove a git worktree at `path` if it looks like a temporary
/// worktree created by superharness (path is under /tmp/ and contains a .git
/// FILE — which is the worktree marker — rather than a .git directory).
///
/// Errors are logged as warnings; the caller's pane kill always proceeds
/// regardless of whether this succeeds.
fn cleanup_worktree(path: &str) {
    // Only act on paths under /tmp/ — we never auto-remove production worktrees.
    if !path.starts_with("/tmp/") {
        return;
    }

    let git_marker = std::path::Path::new(path).join(".git");

    // A worktree has a .git FILE; a normal repo has a .git DIRECTORY.
    if !git_marker.is_file() {
        return;
    }

    // Read the .git file to extract the gitdir path.
    // Format: "gitdir: /path/to/main/.git/worktrees/<name>"
    let content = match std::fs::read_to_string(&git_marker) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[kill] WARNING: could not read {}: {e}",
                git_marker.display()
            );
            return;
        }
    };

    let gitdir = content.lines().find_map(|line| {
        line.trim()
            .strip_prefix("gitdir:")
            .map(|rest| rest.trim().to_string())
    });

    let gitdir = match gitdir {
        Some(p) => p,
        None => {
            eprintln!(
                "[kill] WARNING: malformed .git file (no 'gitdir:' line) at {}",
                git_marker.display()
            );
            return;
        }
    };

    // Resolve to an absolute path (gitdir may be relative to the worktree).
    let gitdir_abs = if std::path::Path::new(&gitdir).is_absolute() {
        std::path::PathBuf::from(&gitdir)
    } else {
        std::path::Path::new(path).join(&gitdir)
    };

    // gitdir_abs is typically  <main_repo>/.git/worktrees/<name>
    // Navigate up three levels:  <name> → worktrees/ → .git/ → <main_repo>
    let main_repo = gitdir_abs
        .parent() // worktrees/<name> → worktrees/
        .and_then(|p| p.parent()) // worktrees/ → .git/
        .and_then(|p| p.parent()); // .git/ → <main_repo>

    let main_repo = match main_repo {
        Some(p) => p.to_string_lossy().to_string(),
        None => {
            eprintln!(
                "[kill] WARNING: could not determine main repo from gitdir '{gitdir}' — skipping worktree cleanup"
            );
            return;
        }
    };

    // Run: git -C <main_repo> worktree remove --force <path>
    match std::process::Command::new("git")
        .args(["-C", &main_repo, "worktree", "remove", "--force", path])
        .output()
    {
        Ok(out) if out.status.success() => {
            eprintln!("[kill] auto-removed git worktree: {path}");
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!("[kill] WARNING: 'git worktree remove --force {path}' failed: {stderr}");
        }
        Err(e) => {
            eprintln!("[kill] WARNING: could not run git worktree remove: {e}");
        }
    }
}

/// Kill a pane, auto-cleaning up any git worktree associated with its working
/// directory when that directory is under /tmp/.
pub fn kill(pane: &str) -> Result<()> {
    // Query the pane's current working directory BEFORE killing it.
    // If we can determine it, attempt worktree cleanup.
    if let Ok(raw) = tmux(&["display-message", "-t", pane, "-p", "#{pane_current_path}"]) {
        let path = raw.trim();
        if !path.is_empty() {
            cleanup_worktree(path);
        }
    }

    tmux_ok(&["kill-pane", "-t", pane])
}

/// Hide a pane to its own background tab.
pub fn hide(pane: &str, name: Option<&str>) -> Result<()> {
    let window_name = name.unwrap_or("worker");
    // Use -s (source pane) not -t (target window) — break-pane expects
    // -s for the pane to break out and -t for the destination window.
    tmux_ok(&["break-pane", "-s", pane, "-d", "-n", window_name])
}

/// Surface a background pane back into the main window.
pub fn show(pane: &str, split: &str) -> Result<()> {
    let flag = if split.starts_with('v') { "-v" } else { "-h" };
    let target = format!("{SESSION}:0");
    tmux_ok(&["join-pane", "-s", pane, "-t", &target, flag, "-d"])?;
    let _ = super::smart_layout();
    Ok(())
}

/// Surface a background pane back into the main window with horizontal split and auto-layout.
/// Equivalent to `show(pane, "h")`.
pub fn surface(pane: &str) -> Result<()> {
    show(pane, "h")
}
