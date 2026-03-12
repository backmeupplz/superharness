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

fn configure_session() -> Result<()> {
    tmux_ok(&["set-option", "-t", SESSION, "allow-set-title", "off"])?;

    // Enable extended keys for modified key combinations (Shift+Enter, etc.)
    tmux_ok(&["set-option", "-s", "extended-keys", "on"])?;
    tmux_ok(&["set-option", "-as", "terminal-features", "*:extkeys"])?;

    // Bind Shift+Enter to send escape sequence that opencode expects for multi-line input
    tmux_ok(&["bind-key", "-n", "S-Enter", "send-keys", "Escape", "[13;2u"])?;

    Ok(())
}

/// Spawn a new opencode worker as a pane in the superharness window.
pub fn spawn(task: &str, dir: &str, name: Option<&str>) -> Result<String> {
    ensure_session()?;

    let abs_dir =
        std::fs::canonicalize(dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    // Create pane with the task ready but hidden
    let cmd = format!("opencode --prompt {}", shell_escape(task));
    let _window_name = name.unwrap_or("worker");

    // Split the current window to create a new pane
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
    ])?;

    // Auto-layout so panes stay usable
    let _ = tmux_ok(&["select-layout", "-t", SESSION, "tiled"]);

    // Clear screen and show splash, then run opencode silently
    // Using stty -echo to hide input, clear screen, show splash, then run opencode
    let setup_cmd = format!(
        "clear && printf '{}' && {}",
        r#"\n\n   ╔═══════════════════════════════════════════════════════╗\n   ║                                                       ║\n   ║         ⚡ SUPERHARNESS - AI Agent Worker ⚡          ║\n   ║                                                       ║\n   ║              Initializing AI Agent...                 ║\n   ║                                                       ║\n   ╚═══════════════════════════════════════════════════════╝\n\n"#,
        cmd
    );

    tmux_ok(&["send-keys", "-t", &pane_id, &setup_cmd, "Enter"])?;

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
        "#{pane_id}\t#{window_name}\t#{pane_current_command}\t#{pane_current_path}",
    ])?;

    let panes = output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            PaneInfo {
                id: parts.first().unwrap_or(&"").to_string(),
                window: parts.get(1).unwrap_or(&"").to_string(),
                command: parts.get(2).unwrap_or(&"").to_string(),
                path: parts.get(3).unwrap_or(&"").to_string(),
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

    // Create session running opencode directly. No shell = no prompt flash.
    // Use a wrapper that shows a centered splash, hides cursor, then execs opencode.
    // The splash stays visible until opencode's TUI takes over the screen.
    let splash = r#"bash -c '
COLS=$(tput cols 2>/dev/null || echo 80)
ROWS=$(tput lines 2>/dev/null || echo 24)
LOGO_H=15; LOGO_W=59
TOP=$(( (ROWS - LOGO_H) / 2 )); LEFT=$(( (COLS - LOGO_W) / 2 ))
[ "$TOP" -lt 0 ] 2>/dev/null && TOP=0
[ "$LEFT" -lt 0 ] 2>/dev/null && LEFT=0
PAD=$(printf "%*s" "$LEFT" "")
printf "\033[2J\033[H\033[?25l"
i=0; while [ "$i" -lt "$TOP" ]; do printf "\n"; i=$((i+1)); done
printf "\033[38;5;214m"
printf "%s%s\n" "$PAD" "███████╗██╗   ██╗██████╗ ███████╗██████╗ "
printf "%s%s\n" "$PAD" "██╔════╝██║   ██║██╔══██╗██╔════╝██╔══██╗"
printf "%s%s\n" "$PAD" "███████╗██║   ██║██████╔╝█████╗  ██████╔╝"
printf "%s%s\n" "$PAD" "╚════██║██║   ██║██╔═══╝ ██╔══╝  ██╔══██╗"
printf "%s%s\n" "$PAD" "███████║╚██████╔╝██║     ███████╗██║  ██║"
printf "%s%s\n" "$PAD" "╚══════╝ ╚═════╝ ╚═╝     ╚══════╝╚═╝  ╚═╝"
printf "\n"
printf "%s%s\n" "$PAD" "██╗  ██╗ █████╗ ██████╗ ███╗   ██╗███████╗███████╗███████╗"
printf "%s%s\n" "$PAD" "██║  ██║██╔══██╗██╔══██╗████╗  ██║██╔════╝██╔════╝██╔════╝"
printf "%s%s\n" "$PAD" "███████║███████║██████╔╝██╔██╗ ██║█████╗  ███████╗███████╗"
printf "%s%s\n" "$PAD" "██╔══██║██╔══██║██╔══██╗██║╚██╗██║██╔══╝  ╚════██║╚════██║"
printf "%s%s\n" "$PAD" "██║  ██║██║  ██║██║  ██║██║ ╚████║███████╗███████║███████║"
printf "%s%s\n" "$PAD" "╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝╚══════╝╚══════╝╚══════╝"
printf "\n"
MSG="Loading orchestrator..."
MSG_LEFT=$(( (COLS - ${#MSG}) / 2 ))
[ "$MSG_LEFT" -lt 0 ] 2>/dev/null && MSG_LEFT=0
printf "\033[38;5;245m%*s%s\033[0m\n" "$MSG_LEFT" "" "$MSG"
exec opencode'"#;
    tmux_ok(&[
        "new-session", "-d", "-s", SESSION, "-c", &dir_str,
        "bash", "-c", splash,
    ])?;
    configure_session()?;

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
