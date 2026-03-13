use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::Command;

use crate::harness;
use crate::layout;
use crate::loop_guard;

const SESSION: &str = "superharness";

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

fn has_session() -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", SESSION])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ensure_session() -> Result<()> {
    if !has_session() {
        tmux_ok(&["new-session", "-d", "-s", SESSION])?;
    }
    Ok(())
}

/// Push all current env vars into the tmux session so spawned panes inherit them.
fn export_env_to_session() -> Result<()> {
    for (key, value) in std::env::vars() {
        // Skip internal/problematic vars
        if key.starts_with('_') || key.contains('=') {
            continue;
        }
        let _ = tmux_ok(&["set-environment", "-t", SESSION, &key, &value]);
    }
    Ok(())
}

fn configure_session(bin_path: &str) -> Result<()> {
    tmux_ok(&["set-option", "-t", SESSION, "allow-set-title", "off"])?;

    // Enable extended keys for modified key combinations (Shift+Enter, etc.)
    tmux_ok(&["set-option", "-s", "extended-keys", "on"])?;
    tmux_ok(&["set-option", "-as", "terminal-features", "*:extkeys"])?;

    // Bind Shift+Enter to send escape sequence that opencode expects for multi-line input
    tmux_ok(&["bind-key", "-n", "S-Enter", "send-keys", "Escape", "[13;2u"])?;

    // Enable pane border status at top and show pane title
    tmux_ok(&["set-option", "-t", SESSION, "pane-border-status", "top"])?;
    tmux_ok(&[
        "set-option",
        "-t",
        SESSION,
        "pane-border-format",
        "#{pane_title}",
    ])?;

    // Bind Ctrl+Backspace to send kitty protocol sequence for 'delete word backwards'.
    // Use -l (literal) so tmux sends the exact bytes without any translation.
    // Also bind C-h as an alias because some terminals send Ctrl+H for Ctrl+Backspace.
    tmux_ok(&[
        "bind-key",
        "-n",
        "C-BSpace",
        "send-keys",
        "-l",
        "\x1b[127;5u",
    ])?;
    tmux_ok(&["bind-key", "-n", "C-h", "send-keys", "-l", "\x1b[127;5u"])?;

    // Bind Ctrl+Left/Right for word navigation (kitty protocol sequences).
    tmux_ok(&["bind-key", "-n", "C-Left", "send-keys", "\x1b[1;5D"])?;
    tmux_ok(&["bind-key", "-n", "C-Right", "send-keys", "\x1b[1;5C"])?;

    // ── Status bar ──────────────────────────────────────────────────────────
    // Store the binary path as a tmux environment variable so bindings can use it.
    tmux_ok(&[
        "set-environment",
        "-t",
        SESSION,
        "SUPERHARNESS_BIN",
        bin_path,
    ])?;

    // Bottom status bar: always on, shows mode / worker count / key hints.
    tmux_ok(&["set-option", "-t", SESSION, "status", "on"])?;
    tmux_ok(&["set-option", "-t", SESSION, "status-position", "bottom"])?;
    tmux_ok(&["set-option", "-t", SESSION, "status-interval", "5"])?;
    tmux_ok(&[
        "set-option",
        "-t",
        SESSION,
        "status-style",
        "bg=#1a2d4a,fg=colour250",
    ])?;

    // Left side: session name label — wrapped in range=window|1 so clicking it
    // navigates back to the main orchestrator window (tmux 3.2+ feature).
    // Also bind MouseDown1StatusLeft as a belt-and-suspenders fallback.
    tmux_ok(&[
        "set-option",
        "-t",
        SESSION,
        "status-left",
        "#[range=window|1]#[bg=colour214,fg=colour232,bold] SUPERHARNESS #[range=default]",
    ])?;
    tmux_ok(&["set-option", "-t", SESSION, "status-left-length", "22"])?;
    // Fallback mouse binding: clicking anywhere in status-left area goes to window 1.
    let _ = tmux_ok(&[
        "bind-key",
        "-n",
        "MouseDown1StatusLeft",
        "select-window",
        "-t",
        ":1",
    ]);

    // Right side: dynamic shell fragments read mode + pane count.
    // Uses grep to extract mode from the project-local .superharness/state.json.
    // Falls back to the global active_project.txt to locate the project dir.
    // The shell snippet produces "AWAY" or "PRESENT" from the state file.
    let mode_snippet = r##"#(p=$(cat $HOME/.local/share/superharness/active_project.txt 2>/dev/null); f="$p/.superharness/state.json"; if [ -f "$f" ]; then m=$(jq -r '.mode' "$f" 2>/dev/null | tr '[:lower:]' '[:upper:]'); [ -z "$m" ] && m=$(grep -o '"mode"[[:space:]]*:[[:space:]]*"[^"]*"' "$f" | grep -o '"[^"]*"$' | tr -d '"' | tr '[:lower:]' '[:upper:]'); [ "$m" = "AWAY" ] && echo "#[fg=colour214,bold]AWAY#[default]" || echo "#[fg=colour71,bold]PRESENT#[default]"; else echo "#[fg=colour71,bold]PRESENT#[default]"; fi)"##;

    // Heartbeat indicator: shows emoji + seconds to next beat.
    // Uses ❤ (U+2764 without variation selector) which is single-width in terminals.
    let heartbeat_snippet = format!("#({bin_path} heartbeat-status 2>/dev/null || echo '❤ --')");

    // Worker count for F4 button label: total worker pane count.
    let worker_count_snippet =
        format!("#({bin_path} status-counts 2>/dev/null | cut -d/ -f2 || echo '0')");

    let status_right = format!(
        "#[fg=colour240]│ #[fg=colour214]MODE:{mode_snippet} \
         #[fg=colour240]│ #[fg=colour196]{heartbeat_snippet} \
         #[fg=colour240]│ #[fg=colour110] F1:toggle-away #[fg=colour240] │ #[fg=colour110] F2:settings #[fg=colour240] │ #[fg=colour110] F3:status #[fg=colour240] │ #[fg=colour110] F4:workers ({worker_count_snippet}) #[fg=colour240] │ #[fg=colour110] F5:tasks #[fg=colour240] │ #[fg=colour110] F6:events  #[default]"
    );

    tmux_ok(&["set-option", "-t", SESSION, "status-right", &status_right])?;
    tmux_ok(&["set-option", "-t", SESSION, "status-right-length", "160"])?;

    // Window status (centre): hide window index/name entirely for a clean bar.
    tmux_ok(&["set-option", "-t", SESSION, "window-status-format", ""])?;
    tmux_ok(&[
        "set-option",
        "-t",
        SESSION,
        "window-status-current-format",
        "",
    ])?;
    tmux_ok(&[
        "set-option",
        "-t",
        SESSION,
        "window-status-current-style",
        "fg=colour214,bold",
    ])?;

    // ── F-key shortcuts (no prefix required) ────────────────────────────────
    // display-popup is a tmux command, not a shell command — use bind-key directly (NOT run-shell).

    // F1 → toggle-mode: sends a mode-switch message directly to the main orchestrator pane (%0)
    tmux_ok(&[
        "bind-key",
        "-n",
        "F1",
        "run-shell",
        &format!("{bin_path} toggle-mode"),
    ])?;

    // F2 → harness-settings: interactive popup to view/change the default harness & model
    tmux_ok(&[
        "bind-key",
        "-n",
        "F2",
        "display-popup",
        "-E",
        "-b",
        "rounded",
        "-w",
        "70",
        "-h",
        "22",
        &format!("{bin_path} harness-settings"),
    ])?;

    // F3 → status-human (mode + pending decisions + worker health, human-readable)
    tmux_ok(&[
        "bind-key",
        "-n",
        "F3",
        "display-popup",
        "-E",
        "-b",
        "rounded",
        "-w",
        "110",
        "-h",
        "42",
        &format!(
            "{bin_path} status-human 2>&1; echo; echo '  Press any key to close...'; read -n1"
        ),
    ])?;

    // F4 → workers (human-readable worker list)
    tmux_ok(&[
        "bind-key",
        "-n",
        "F4",
        "display-popup",
        "-E",
        "-b",
        "rounded",
        "-w",
        "110",
        "-h",
        "36",
        &format!("{bin_path} workers 2>&1; echo; echo '  Press any key to close...'; read -n1"),
    ])?;

    // F5 → tasks-modal (task list grouped by status, scrollable via less)
    tmux_ok(&[
        "bind-key",
        "-n",
        "F5",
        "display-popup",
        "-E",
        "-b",
        "rounded",
        "-w",
        "110",
        "-h",
        "42",
        &format!("bash -c '{bin_path} tasks-modal 2>&1 | less -R'"),
    ])?;

    // F6 → event-feed (scrollable event log via less; press q to close)
    tmux_ok(&[
        "bind-key",
        "-n",
        "F6",
        "display-popup",
        "-E",
        "-b",
        "rounded",
        "-w",
        "110",
        "-h",
        "42",
        &format!("bash -c '{bin_path} event-feed 2>&1 | less -R'"),
    ])?;

    Ok(())
}

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
    ensure_session()?;

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
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
        .join("superharness");
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
        let _ = smart_layout();
        let _ = auto_compact();
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

/// Send a bare Enter keypress to a pane (no text).
pub fn send_raw(pane: &str, _text: &str) -> Result<()> {
    tmux_ok(&["send-keys", "-t", pane, "Enter"])
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
    if !has_session() {
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
    let _ = smart_layout();
    Ok(())
}

/// Surface a background pane back into the main window with horizontal split and auto-layout.
/// Equivalent to `show(pane, "h")`.
pub fn surface(pane: &str) -> Result<()> {
    show(pane, "h")
}

/// Select window 0 (the main orchestrator window) so that %0 is visible
/// after a worker finishes or is cleaned up.
pub fn select_orchestrator() -> Result<()> {
    tmux_ok(&["select-window", "-t", &format!("{SESSION}:0")])
}

/// Auto-compact the main window: if more than 4 worker panes are visible,
/// move excess panes (highest pane_index, never %0) to background tabs.
/// Called automatically after each `spawn`.
pub fn auto_compact() -> Result<()> {
    // List panes in main window (window 0) with their indices and titles
    let output = match tmux(&[
        "list-panes",
        "-t",
        &format!("{SESSION}:0"),
        "-F",
        "#{pane_id}\t#{pane_index}\t#{pane_title}",
    ]) {
        Ok(o) => o,
        Err(_) => return Ok(()), // Session or window not available yet
    };

    let mut panes: Vec<(String, u32, String)> = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let id = parts[0].to_string();
        let index: u32 = parts[1].parse().unwrap_or(0);
        let title = parts[2].to_string();
        panes.push((id, index, title));
    }

    // Exclude %0 (orchestrator), sort remaining by pane_index ascending
    let mut workers: Vec<(String, u32, String)> =
        panes.into_iter().filter(|(id, _, _)| id != "%0").collect();
    workers.sort_by_key(|(_, idx, _)| *idx);

    // Dynamic threshold based on terminal width
    let (term_w, _) = get_terminal_size();
    let max_workers_visible = layout::max_workers_visible(term_w);

    if workers.len() > max_workers_visible {
        let excess = workers.len() - max_workers_visible;
        // Move the highest-index panes (last in sorted order) to background
        let to_move: Vec<_> = workers.into_iter().rev().take(excess).collect();
        for (id, _, title) in to_move {
            let tab_name: String = title.chars().take(20).collect();
            let tab_name = tab_name.trim().to_string();
            let tab_name = if tab_name.is_empty() {
                "worker".to_string()
            } else {
                tab_name
            };
            let _ = tmux_ok(&["break-pane", "-s", &id, "-d", "-n", &tab_name]);
        }
    }

    // Enforce minimum readable pane size after compaction.
    let _ = layout::enforce_min_pane_size();

    Ok(())
}

/// Information returned by the `terminal-size` subcommand.
#[derive(Serialize)]
pub struct TerminalSizeInfo {
    pub width: u32,
    pub height: u32,
    /// Fixed row count for the orchestrator pane (%0).
    pub main_pane_rows: u32,
    /// Number of non-%0 panes currently visible in window 0.
    pub workers_visible: usize,
    /// Recommended maximum number of workers to show simultaneously,
    /// based on terminal width:
    ///   < 120  → 1
    ///   120–200 → 2
    ///   200–300 → 3
    ///   300+   → 4
    pub recommended_max_workers: usize,
}

/// Compute terminal size metadata and return it as a [`TerminalSizeInfo`].
pub fn terminal_size_info() -> TerminalSizeInfo {
    let (width, height) = get_terminal_size();

    // Count non-%0 panes in window 0.
    let workers_visible: usize = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            &format!("{SESSION}:0"),
            "-F",
            "#{pane_id}",
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty() && *l != "%0")
                .count()
        })
        .unwrap_or(0);

    let recommended_max_workers = match width {
        w if w >= 300 => 4,
        w if w >= 200 => 3,
        w if w >= 120 => 2,
        _ => 1,
    };

    TerminalSizeInfo {
        width,
        height,
        main_pane_rows: 82,
        workers_visible,
        recommended_max_workers,
    }
}

/// Query the current tmux window dimensions.
/// Returns `Some((width, height))` on success, `None` if tmux is unavailable
/// or the session does not exist.  Unlike [`get_terminal_size`] this never
/// fabricates a fallback value.
pub fn terminal_size() -> Option<(u32, u32)> {
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-t",
            SESSION,
            "-p",
            "#{window_width} #{window_height}",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let s = String::from_utf8_lossy(&output.stdout);
    let s = s.trim();
    let mut parts = s.split_whitespace();
    let w: u32 = parts.next()?.parse().ok()?;
    let h: u32 = parts.next()?.parse().ok()?;
    Some((w, h))
}

/// Query the current tmux window dimensions dynamically.
/// Returns (width, height). Falls back to (80, 24) on any error.
pub fn get_terminal_size() -> (u32, u32) {
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-t",
            SESSION,
            "-p",
            "#{window_width} #{window_height}",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            let s = s.trim();
            let mut parts = s.split_whitespace();
            let w: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(80);
            let h: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(24);
            (w, h)
        }
        _ => (80, 24),
    }
}

/// Compact the main window: move any pane (except %0) that is too small
/// (width < term_w/3 or height < term_h/3) to a background tab.
/// Returns (moved_count, remaining_visible_count).
pub fn compact_panes() -> Result<(usize, usize)> {
    let (term_w, term_h) = get_terminal_size();

    // List all panes across all windows with dimensions and window index
    let output = match tmux(&[
        "list-panes",
        "-t",
        SESSION,
        "-a",
        "-F",
        "#{pane_id}\t#{pane_width}\t#{pane_height}\t#{window_index}\t#{pane_title}",
    ]) {
        Ok(o) => o,
        Err(_) => return Ok((0, 0)),
    };

    let mut to_move: Vec<(String, String)> = Vec::new(); // (id, tab_name)
    let mut remaining = 0usize;

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(5, '\t').collect();
        if parts.len() < 5 {
            continue;
        }
        let id = parts[0];
        let width: u32 = parts[1].parse().unwrap_or(0);
        let height: u32 = parts[2].parse().unwrap_or(0);
        let window_index: u32 = parts[3].parse().unwrap_or(999);
        let title = parts[4];

        // Only process panes in the main window (window 0), skip orchestrator %0
        if window_index != 0 || id == "%0" {
            continue;
        }

        // Use layout-engine aware thresholds: a pane is "too small" when its
        // width is less than what the strategy would allocate per worker, or
        // its height is less than 1/4 of the terminal height.
        let min_w = term_w / (layout::max_workers_visible(term_w) as u32 + 1);
        let min_h = term_h / 4;
        if width < min_w || height < min_h {
            let tab_name: String = title.chars().take(20).collect();
            let tab_name = tab_name.trim().to_string();
            let tab_name = if tab_name.is_empty() {
                "worker".to_string()
            } else {
                tab_name
            };
            to_move.push((id.to_string(), tab_name));
        } else {
            remaining += 1;
        }
    }

    let mut moved = 0usize;
    for (id, tab_name) in &to_move {
        if tmux_ok(&["break-pane", "-s", id, "-d", "-n", tab_name]).is_ok() {
            moved += 1;
        }
    }

    // Re-apply smart layout to main window if anything was moved
    if moved > 0 {
        let _ = smart_layout();
    }

    // Enforce minimum readable pane size regardless of whether compaction moved anything.
    let _ = layout::enforce_min_pane_size();

    Ok((moved, remaining))
}

/// Resize a pane.
pub fn resize(pane: &str, direction: &str, amount: u32) -> Result<()> {
    let flag = match direction.to_uppercase().as_str() {
        "U" => "-U",
        "D" => "-D",
        "L" => "-L",
        "R" => "-R",
        other => anyhow::bail!("invalid direction: {other} (use U, D, L, R)"),
    };
    tmux_ok(&["resize-pane", "-t", pane, flag, &amount.to_string()])
}

/// Apply a layout preset.
pub fn layout(name: &str) -> Result<()> {
    tmux_ok(&["select-layout", "-t", SESSION, name])
}

// ---------------------------------------------------------------------------
// Smart layout helpers
// ---------------------------------------------------------------------------

/// Build a [`layout::PaneLayout`] list from the panes currently visible in
/// the main window (window 0), with no pane flagged as needing attention.
fn main_window_pane_layouts() -> Vec<layout::PaneLayout> {
    let output = match tmux(&[
        "list-panes",
        "-t",
        &format!("{SESSION}:0"),
        "-F",
        "#{pane_id}\t#{pane_title}",
    ]) {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            let id = parts.first().unwrap_or(&"").to_string();
            let title = parts.get(1).unwrap_or(&"").to_string();
            let is_orch = id == "%0";
            layout::PaneLayout {
                priority: if is_orch { 255 } else { 0 },
                is_orchestrator: is_orch,
                needs_attention: false,
                id,
                title,
            }
        })
        .collect()
}

/// Apply the smart layout to the current main window without any special
/// attention pane.  Called after `spawn`, `show`/`surface`, and `compact_panes`.
pub fn smart_layout() -> Result<()> {
    let (term_w, term_h) = get_terminal_size();
    let panes = main_window_pane_layouts();
    let engine = layout::LayoutEngine::new(term_w, term_h, panes);
    engine.apply()?;
    // Enforce minimum readable pane size after every layout change.
    let _ = layout::enforce_min_pane_size();
    Ok(())
}

/// Apply the smart layout, treating `attention_pane` as needing extra space.
/// If `attention_pane` is currently in a background tab it is surfaced first.
///
/// This is the primary entry point used by `watch.rs` when a pane is detected
/// as `Waiting` (i.e. asking a question or waiting for permission).
pub fn smart_layout_with_attention(attention_pane: Option<&str>) -> Result<()> {
    // 1. Surface the pane if it is in a background window.
    if let Some(pane) = attention_pane {
        let main_panes = main_window_pane_layouts();
        let is_in_main = main_panes.iter().any(|p| p.id == pane);
        if !is_in_main {
            eprintln!("[layout] surfacing attention pane {pane} from background to main window");
            let _ = surface(pane);
        }
    }

    // 2. Build pane list with the attention pane flagged.
    let (term_w, term_h) = get_terminal_size();
    let panes: Vec<layout::PaneLayout> = main_window_pane_layouts()
        .into_iter()
        .map(|mut p| {
            if attention_pane.map(|ap| ap == p.id).unwrap_or(false) {
                p.needs_attention = true;
                p.priority = 200;
            }
            p
        })
        .collect();

    let engine = layout::LayoutEngine::new(term_w, term_h, panes);
    engine.apply()
}

/// Start the superharness session with an orchestrator opencode and attach.
pub fn init(dir: &str, bin_path: &str) -> Result<()> {
    let abs_dir =
        std::fs::canonicalize(dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    if has_session() {
        let _ = tmux_ok(&["kill-session", "-t", SESSION]);
    }

    // ── Determine initial prompt BEFORE launching opencode ───────────────────
    // This lets us pass the prompt directly via --prompt rather than using
    // send-keys after the fact (which is unreliable for long/multi-line messages).
    let config_dir_base = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
        .join("superharness");
    let config_path = config_dir_base.join("config.json");

    // ── First-launch harness picker ──────────────────────────────────────────
    // If no config exists yet, detect installed harnesses:
    //   • exactly one found  → silently write it as the default
    //   • multiple found     → show an interactive arrow-key picker so the user
    //                          can choose BEFORE the tmux session opens
    //   • none found         → skip (the AI will surface an error when spawning)
    if !config_path.exists() {
        let candidates = harness::detect_installed();
        match candidates.len() {
            0 => {} // nothing to do — let the AI handle missing harness errors
            1 => {
                // Single harness: silently persist so subsequent sessions skip this.
                let _ = harness::set_default_harness(&config_dir_base, &candidates[0].name);
            }
            _ => {
                // Multiple harnesses: let the user pick before we launch.
                println!();
                println!("  \x1b[1mSuperHarness — first run\x1b[0m");
                println!();
                match harness::run_interactive_picker(&candidates, None) {
                    Ok(Some(chosen)) => {
                        if let Err(e) = harness::set_default_harness(&config_dir_base, &chosen) {
                            eprintln!("warning: could not persist harness choice: {e}");
                        }
                    }
                    Ok(None) => {
                        // User cancelled — the AI will ask during first-run prompt.
                    }
                    Err(e) => {
                        eprintln!("warning: picker error ({e}); the AI will ask instead.");
                    }
                }
            }
        }
    }

    // Read default_model from config so the orchestrator uses the user's preferred model.
    let (default_model, orch_harness): (Option<String>, String) = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        let model = v["default_model"].as_str().map(String::from);
        let config_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
        let h = harness::resolve_harness(config_dir).unwrap_or_else(|_| "opencode".to_string());
        (model, h)
    } else {
        // No config yet: detect what's available for the first-run harness list
        let h = harness::detect_installed()
            .into_iter()
            .next()
            .map(|i| i.binary)
            .unwrap_or_else(|| "opencode".to_string());
        (None, h)
    };

    // auto_submit = true  → pass --prompt to harness (it submits immediately)
    // auto_submit = false → launch harness without --prompt and prefill the input
    //                       via a background tmux send-keys (no Enter) so the user
    //                       can review and edit before sending.

    // Build harness-appropriate model-listing command for first-run guidance
    let harness_models_cmd = match orch_harness.as_str() {
        "claude" => "`claude --help` to see available options",
        "codex" => "`codex --help` to see available options",
        _ => "`opencode models` to see all available models, and `opencode auth list` to see authenticated providers",
    };

    // Detect whether multiple harnesses are available for first-run harness selection
    let installed_harnesses = harness::detect_installed();
    let harness_selection_prompt = if installed_harnesses.len() > 1 && !config_path.exists() {
        let names: Vec<String> = installed_harnesses
            .iter()
            .map(|h| format!("{} ({})", h.display_name, h.binary))
            .collect();
        format!(
            " Multiple AI harnesses are installed: {}. \
             Ask the user which one they prefer to use as the default for spawning workers. \
             Save their preference as 'default_harness' in the config.",
            names.join(", ")
        )
    } else {
        String::new()
    };

    let (initial_prompt, _auto_submit): (String, bool) = if !config_path.exists() {
        // First-run: ask model to set up preferences (auto-submit is fine here)
        let config_path_str = config_path.to_string_lossy().to_string();
        (
            format!(
            "[SUPERHARNESS FIRST RUN] Welcome! Before we start, please set up model preferences. \
            Run {harness_models_cmd}. \
            Then ask the user: which provider they prefer, and which \
            model should be the default when spawning workers.{harness_selection_prompt} \
            Keep it conversational — just a couple of questions. \
            Once you have their answers, write the config to {config_path_str} \
            as JSON with fields: default_model (string), default_harness (string, optional), \
            preferred_providers (array of strings), preferred_models (array of strings). \
            Create the directory if needed. After saving, \
            confirm it's done and ask what they'd like to work on today."
        ),
            true,
        )
    } else {
        let state_file = std::path::PathBuf::from(&dir_str)
            .join(".superharness")
            .join("state.json");
        let tasks_file = std::path::PathBuf::from(&dir_str)
            .join(".superharness")
            .join("tasks.json");
        let decisions_file = std::path::PathBuf::from(&dir_str)
            .join(".superharness")
            .join("decisions.json");

        let has_state = state_file.exists();
        let tasks_content_raw = if tasks_file.exists() {
            std::fs::read_to_string(&tasks_file).unwrap_or_default()
        } else {
            String::new()
        };
        let tasks_empty = {
            let trimmed = tasks_content_raw.trim();
            trimmed.is_empty() || trimmed == "[]" || trimmed == "null"
        };

        let tasks_file_path = tasks_file.to_string_lossy().to_string();
        if !has_state || tasks_empty {
            // Planning mode: prefill the prompt but let the user submit manually.
            (format!(
                "[SUPERHARNESS PLANNING] No project plan found for this directory ({dir_str}). \
                Please start a planning conversation with the user: \
                1. Ask what they want to build or what the goal of this project is. \
                2. Ask clarifying questions to understand scope, constraints, and priorities. \
                3. Break the goal down into concrete tasks. \
                4. Identify which tasks can run in parallel and which depend on each other. \
                5. Write the resulting tasks to {tasks_file_path} (create .superharness/ dir if needed). \
                6. Once the plan is captured, confirm it with the user and ask if they want to start immediately. \
                Be conversational — this is a planning chat, not a form to fill out."
            ), false)
        } else {
            // Resume mode: inject previous context and auto-submit.
            // Tasks are NOT inlined here — the orchestrator reads them fresh from disk to avoid
            // working from a stale startup-time snapshot.
            let state_content =
                std::fs::read_to_string(&state_file).unwrap_or_else(|_| "{}".to_string());
            let decisions_content = if decisions_file.exists() {
                std::fs::read_to_string(&decisions_file).unwrap_or_else(|_| "none".to_string())
            } else {
                "none".to_string()
            };
            (format!(
                "[SUPERHARNESS CONTEXT] Resuming session. Previous state: {}. \
                Tasks file: {} — please read this file to see current tasks. \
                Decisions pending: {}. \
                Please acknowledge this state and continue from where you left off, or ask the user what they want to work on.",
                state_content,
                tasks_file_path,
                decisions_content,
            ), true)
        }
    };

    // Get current terminal size (this is the real terminal we'll attach to)
    let (cols, rows): (i32, i32) = term_size::dimensions()
        .map(|(c, r)| (c as i32, r as i32))
        .unwrap_or((80, 24));

    // Subtract 1 row for tmux status bar
    let rows = rows - 1;
    let logo_h = 15i32;
    let logo_w = 59i32;
    let top = ((rows - logo_h) / 2).max(0);
    let left = ((cols - logo_w) / 2).max(0);
    let msg = "Loading orchestrator...";
    let msg_left = ((cols - msg.len() as i32) / 2).max(0);

    let p = " ".repeat(left as usize);
    let top_nl = "\n".repeat(top as usize);
    let mp = " ".repeat(msg_left as usize);

    let logo_lines = [
        "███████╗██╗   ██╗██████╗ ███████╗██████╗ ",
        "██╔════╝██║   ██║██╔══██╗██╔════╝██╔══██╗",
        "███████╗██║   ██║██████╔╝█████╗  ██████╔╝",
        "╚════██║██║   ██║██╔═══╝ ██╔══╝  ██╔══██╗",
        "███████║╚██████╔╝██║     ███████╗██║  ██║",
        "╚══════╝ ╚═════╝ ╚═╝     ╚══════╝╚═╝  ╚═╝",
        "",
        "██╗  ██╗ █████╗ ██████╗ ███╗   ██╗███████╗███████╗███████╗",
        "██║  ██║██╔══██╗██╔══██╗████╗  ██║██╔════╝██╔════╝██╔════╝",
        "███████║███████║██████╔╝██╔██╗ ██║█████╗  ███████╗███████╗",
        "██╔══██║██╔══██║██╔══██╗██║╚██╗██║██╔══╝  ╚════██║╚════██║",
        "██║  ██║██║  ██║██║  ██║██║ ╚████║███████╗███████║███████║",
        "╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝╚══════╝╚══════╝╚══════╝",
    ];
    let logo_text: String = logo_lines
        .iter()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{p}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Launch the harness with --prompt / --print / positional arg to pre-fill and submit
    // the initial message.  build_harness_cmd handles per-harness flag differences.
    let opencode_cmd =
        harness::build_harness_cmd(&orch_harness, default_model.as_deref(), &initial_prompt);

    let splash = format!(
        "printf '\\033[2J\\033[H\\033[?25l{top_nl}\\033[38;5;214m{logo_text}\\n\\n\\033[38;5;245m{mp}{msg}\\033[0m'; exec {opencode_cmd}"
    );

    tmux_ok(&["new-session", "-d", "-s", SESSION, "-c", &dir_str])?;
    configure_session(bin_path)?;
    export_env_to_session()?;

    // Replace default shell with splash+opencode.
    // Use bash -lc (login shell) so that ~/.profile and ~/.bash_profile are
    // sourced, ensuring PATH and credential env vars are fully initialised.
    tmux_ok(&["respawn-pane", "-t", SESSION, "-k", "bash", "-lc", &splash])?;

    let status = Command::new("tmux")
        .args(["attach-session", "-t", SESSION])
        .status()
        .context("failed to attach to tmux session")?;

    // Clean up when we return (user detached or opencode exited)
    if has_session() {
        let _ = tmux_ok(&["kill-session", "-t", SESSION]);
    }

    if !status.success() {
        bail!("tmux attach failed");
    }

    Ok(())
}
