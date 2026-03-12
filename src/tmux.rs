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

/// Spawn a new opencode worker in its own tmux window (tab).
pub fn spawn(task: &str, dir: &str, name: Option<&str>) -> Result<String> {
    ensure_session()?;

    let abs_dir =
        std::fs::canonicalize(dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    let window_name = name.unwrap_or("worker");

    // Create a new window (tab) in the session
    let pane_id = tmux(&[
        "new-window",
        "-t",
        SESSION,
        "-n",
        window_name,
        "-d", // don't switch to it
        "-P", // print target pane
        "-F",
        "#{pane_id}",
        "-c",
        &dir_str,
    ])?;

    // Launch opencode with the task pre-filled
    let cmd = format!("opencode --prompt {}", shell_escape(task));
    tmux_ok(&["send-keys", "-t", &pane_id, &cmd, "Enter"])?;

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

/// Start the superharness session with an orchestrator opencode and attach.
pub fn init(dir: &str) -> Result<()> {
    let abs_dir =
        std::fs::canonicalize(dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    if has_session() {
        let _ = tmux_ok(&["kill-session", "-t", SESSION]);
    }

    tmux_ok(&["new-session", "-d", "-s", SESSION, "-c", &dir_str])?;
    configure_session()?;
    tmux_ok(&["send-keys", "-t", SESSION, "opencode", "Enter"])?;

    let status = Command::new("tmux")
        .args(["attach-session", "-t", SESSION])
        .status()
        .context("failed to attach to tmux session")?;

    if !status.success() {
        bail!("tmux attach failed");
    }

    Ok(())
}
