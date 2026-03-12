use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::process::Command;

use crate::loop_guard;
use crate::state::StateManager;

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
    // Must be a single string containing the raw ESC byte so tmux forwards it as one
    // unambiguous CSI sequence (\x1b[127;5u) instead of two separate keystrokes.
    tmux_ok(&["bind-key", "-n", "C-BSpace", "send-keys", "\x1b[127;5u"])?;

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
        "bg=colour235,fg=colour250",
    ])?;

    // Left side: static session name label.
    tmux_ok(&[
        "set-option",
        "-t",
        SESSION,
        "status-left",
        "#[fg=colour214,bold] SUPERHARNESS #[fg=colour240]│ ",
    ])?;
    tmux_ok(&["set-option", "-t", SESSION, "status-left-length", "22"])?;

    // Right side: dynamic shell fragments read mode + pane count.
    // #{E:SUPERHARNESS_BIN} expands the env var we just set.
    // The shell snippet produces "AWAY" or "PRESENT" from the state file.
    let mode_snippet = r#"#(sh -c '
        f="$HOME/.local/share/superharness/state.json"
        if [ -f "$f" ]; then
            m=$(python3 -c "import sys,json; d=json.load(open(\"$f\")); print(d.get(\"mode\",\"present\").upper())" 2>/dev/null)
            echo "${m:-PRESENT}"
        else
            echo PRESENT
        fi
    ')#"#;

    // Pane count (excluding the orchestrator pane 0 by counting all panes minus 1, min 0).
    let pane_count_snippet =
        "#(tmux list-panes -t superharness -a 2>/dev/null | wc -l | tr -d ' ')#";

    let status_right = format!(
        "#[fg=colour240]│ #[fg=colour33]MODE:{mode_snippet} \
         #[fg=colour240]│ #[fg=colour71]PANES:{pane_count_snippet} \
         #[fg=colour240]│ #[fg=colour243] F1:away F2:present F3:status F4:health F5:workers #[default]"
    );

    tmux_ok(&["set-option", "-t", SESSION, "status-right", &status_right])?;
    tmux_ok(&["set-option", "-t", SESSION, "status-right-length", "90"])?;

    // Window status (centre): show window list naturally.
    tmux_ok(&[
        "set-option",
        "-t",
        SESSION,
        "window-status-current-style",
        "fg=colour214,bold",
    ])?;

    // ── F-key shortcuts (no prefix required) ────────────────────────────────
    // F1 → superharness away  (run in a popup so output is visible)
    let f1_cmd = format!(
        "display-popup -E -w 60 -h 12 '{bin_path} away 2>&1; echo; echo Press any key to close...; read -n1'",
        bin_path = bin_path
    );
    tmux_ok(&["bind-key", "-n", "F1", "run-shell", &f1_cmd])?;

    // F2 → superharness present
    let f2_cmd = format!(
        "display-popup -E -w 80 -h 24 '{bin_path} present 2>&1; echo; echo Press any key to close...; read -n1'",
        bin_path = bin_path
    );
    tmux_ok(&["bind-key", "-n", "F2", "run-shell", &f2_cmd])?;

    // F3 → superharness status
    let f3_cmd = format!(
        "display-popup -E -w 80 -h 24 '{bin_path} status 2>&1; echo; echo Press any key to close...; read -n1'",
        bin_path = bin_path
    );
    tmux_ok(&["bind-key", "-n", "F3", "run-shell", &f3_cmd])?;

    // F4 → superharness healthcheck (one-shot health of all panes)
    let f4_cmd = format!(
        "display-popup -E -w 100 -h 30 '{bin_path} healthcheck 2>&1; echo; echo Press any key to close...; read -n1'",
        bin_path = bin_path
    );
    tmux_ok(&["bind-key", "-n", "F4", "run-shell", &f4_cmd])?;

    // F5 → superharness list  (worker pane list)
    let f5_cmd = format!(
        "display-popup -E -w 100 -h 30 '{bin_path} list 2>&1; echo; echo Press any key to close...; read -n1'",
        bin_path = bin_path
    );
    tmux_ok(&["bind-key", "-n", "F5", "run-shell", &f5_cmd])?;

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
pub fn spawn(
    task: &str,
    dir: &str,
    name: Option<&str>,
    model: Option<&str>,
    mode: Option<&str>,
) -> Result<String> {
    ensure_session()?;

    let abs_dir =
        std::fs::canonicalize(dir).with_context(|| format!("invalid directory: {dir}"))?;
    let dir_str = abs_dir.to_string_lossy().to_string();

    let effective_mode = mode.unwrap_or("build");

    let model_flag = match model {
        Some(m) => format!(" --model {}", shell_escape(m)),
        None => String::new(),
    };

    // In plan mode, prefix the task to instruct the agent not to make changes.
    let effective_task = match effective_mode {
        "plan" => format!("[PLAN MODE - do not make changes, only analyze and plan]: {task}"),
        _ => task.to_string(),
    };

    // The opencode command itself (no auto-kill wrapper yet — we need the pane ID first).
    let opencode_cmd = format!(
        "opencode{model_flag} --prompt {}",
        shell_escape(&effective_task)
    );

    // We wrap opencode so that when it exits the pane auto-kills itself.
    // The wrapper uses tmux display-message to get the pane's own ID at runtime,
    // then invokes `superharness kill --pane <id>` after opencode finishes.
    // We resolve the superharness binary path at spawn time so the worker pane
    // can always find it even if PATH is different inside the new bash session.
    let sh_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "superharness".to_string());

    let cmd =
        format!("{opencode_cmd} ; {sh_bin} kill --pane $(tmux display-message -p '#{{pane_id}}')");

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

/// Check whether superharness is currently in away mode.
#[allow(dead_code)]
pub fn is_away() -> bool {
    StateManager::new().map(|sm| sm.is_away()).unwrap_or(false)
}

/// Queue a decision for the human to resolve when they return (away mode).
/// Returns the decision ID on success.
pub fn queue_decision(pane: &str, question: &str, context: &str) -> Result<String> {
    let sm = StateManager::new()?;
    sm.add_pending_decision(pane, question, context)
}

/// Start the superharness session with an orchestrator opencode and attach.
pub fn init(dir: &str, bin_path: &str) -> Result<()> {
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
    configure_session(bin_path)?;
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
