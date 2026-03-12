use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::Command;

const SESSION: &str = "superharness";

/// Escape a string for safe use in a shell command
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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

fn configure_session() -> Result<()> {
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

    Ok(())
}

/// Color palette for pane backgrounds (ANSI 256-color indices, distinct colors)
const PANE_COLORS: &[u8] = &[
    24,  // dark blue
    22,  // dark green
    52,  // dark red
    54,  // dark purple
    58,  // dark olive/yellow
    23,  // dark teal
    88,  // dark crimson
    130, // dark orange
];

/// Spawn a new opencode worker as a pane in the superharness window.
pub fn spawn(task: &str, dir: &str, name: Option<&str>, model: Option<&str>) -> Result<String> {
    ensure_session()?;

    let abs_dir =
        std::fs::canonicalize(dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    let model_flag = match model {
        Some(m) => format!(" --model {}", shell_escape(m)),
        None => String::new(),
    };
    let cmd = format!("opencode{model_flag} --prompt {}", shell_escape(task));

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

    // Determine pane title: use name if provided, otherwise first 30 chars of task
    let title = match name {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            let truncated: String = task.chars().take(30).collect();
            truncated
        }
    };

    // Set pane title
    let _ = tmux_ok(&["select-pane", "-t", &pane_id, "-T", &title]);

    // Get pane index to pick a color from the palette
    let pane_index_str =
        tmux(&["display-message", "-t", &pane_id, "-p", "#{pane_index}"]).unwrap_or_default();
    let pane_index: usize = pane_index_str.trim().parse().unwrap_or(0);
    let color = PANE_COLORS[pane_index % PANE_COLORS.len()];
    let style = format!("bg=colour{color}");
    let _ = tmux_ok(&["select-pane", "-t", &pane_id, "-P", &style]);

    // Auto-layout so panes stay usable
    let _ = tmux_ok(&["select-layout", "-t", SESSION, "tiled"]);

    Ok(pane_id)
}

/// Read recent output from a pane.
pub fn read(pane: &str, lines: u32) -> Result<String> {
    tmux(&["capture-pane", "-t", pane, "-p", "-S", &format!("-{lines}")])
}

/// Send text to a pane.
pub fn send(pane: &str, text: &str) -> Result<()> {
    tmux_ok(&["send-keys", "-t", pane, text, "Enter"])
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

/// Kill a pane.
pub fn kill(pane: &str) -> Result<()> {
    tmux_ok(&["kill-pane", "-t", pane])
}

/// Hide a pane to its own background tab.
pub fn hide(pane: &str, name: Option<&str>) -> Result<()> {
    let window_name = name.unwrap_or("worker");
    tmux_ok(&["break-pane", "-t", pane, "-d", "-n", window_name])
}

/// Surface a background pane back into the main window.
pub fn show(pane: &str, split: &str) -> Result<()> {
    let flag = if split.starts_with('v') { "-v" } else { "-h" };
    let target = format!("{SESSION}:0");
    tmux_ok(&["join-pane", "-s", pane, "-t", &target, flag, "-d"])?;
    let _ = tmux_ok(&["select-layout", "-t", SESSION, "tiled"]);
    Ok(())
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

/// Start the superharness session with an orchestrator opencode and attach.
pub fn init(dir: &str) -> Result<()> {
    let abs_dir =
        std::fs::canonicalize(dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    if has_session() {
        let _ = tmux_ok(&["kill-session", "-t", SESSION]);
    }

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

    let splash = format!(
        "printf '\\033[2J\\033[H\\033[?25l{top_nl}\\033[38;5;214m{logo_text}\\n\\n\\033[38;5;245m{mp}{msg}\\033[0m'; exec opencode"
    );

    tmux_ok(&["new-session", "-d", "-s", SESSION, "-c", &dir_str])?;
    configure_session()?;
    export_env_to_session()?;

    // Replace default shell with splash+opencode
    tmux_ok(&["respawn-pane", "-t", SESSION, "-k", "bash", "-c", &splash])?;

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
