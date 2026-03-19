//! Harness detection and selection.
//!
//! Supported harnesses and their CLI interfaces:
//!
//! Interactive (orchestrator — stays alive for follow-up messages):
//!   - opencode:  opencode [--model <m>] --prompt <task>
//!   - claude:    claude [--model <m>] <task>
//!   - codex:     codex [--model <m>] <task>
//!
//! One-shot (workers — process once and exit):
//!   - opencode:  opencode [--model <m>] --prompt <task>
//!   - claude:    claude -p [--model <m>] <task>
//!   - codex:     codex exec [--model <m>] <task>

use anyhow::{bail, Result};
use std::path::Path;

use crate::util::shell_escape;

/// Metadata about a detected AI coding harness.
#[derive(Debug, Clone)]
pub struct HarnessInfo {
    /// Short identifier used in config (e.g. "opencode", "claude", "codex")
    pub name: String,
    /// Binary name on PATH (currently same as name for all known harnesses)
    pub binary: String,
    /// Human-readable display name
    pub display_name: String,
}

/// Candidate harnesses in discovery-priority order.
const CANDIDATES: &[(&str, &str, &str)] = &[
    ("opencode", "opencode", "OpenCode"),
    ("claude", "claude", "Claude Code"),
    ("codex", "codex", "OpenAI Codex"),
];

/// Check which of the known harnesses are installed on PATH.
/// Returns them in the order they are found.
pub fn detect_installed() -> Vec<HarnessInfo> {
    let mut installed = Vec::new();
    for (name, binary, display_name) in CANDIDATES {
        if binary_on_path(binary) {
            installed.push(HarnessInfo {
                name: name.to_string(),
                binary: binary.to_string(),
                display_name: display_name.to_string(),
            });
        }
    }
    installed
}

/// Return all known candidate harnesses regardless of installation status.
/// Used by the settings picker so users can see all options.
pub fn detect_all_candidates() -> Vec<HarnessInfo> {
    CANDIDATES
        .iter()
        .map(|(name, binary, display_name)| HarnessInfo {
            name: name.to_string(),
            binary: binary.to_string(),
            display_name: display_name.to_string(),
        })
        .collect()
}

/// Return true if `binary` is available on the current PATH.
fn binary_on_path(binary: &str) -> bool {
    std::process::Command::new("which")
        .arg(binary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Read `default_harness` from `<config_dir>/config.json`.
/// Returns `None` if the file does not exist or the field is absent.
pub fn get_default_harness(config_dir: &Path) -> Option<String> {
    let config_path = config_dir.join("config.json");
    if !config_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&config_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v["default_harness"].as_str().map(String::from)
}

/// Write `default_harness` to `<config_dir>/config.json`.
/// Merges with any existing fields so nothing else is overwritten.
pub fn set_default_harness(config_dir: &Path, harness: &str) -> Result<()> {
    let config_path = config_dir.join("config.json");

    // Read existing config or start with an empty object
    let mut v: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure the config directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    v["default_harness"] = serde_json::json!(harness);

    std::fs::write(&config_path, serde_json::to_string_pretty(&v)?)?;

    Ok(())
}

/// Determine which harness binary to use, applying this priority:
///
/// 1. `default_harness` in `<config_dir>/config.json` (if installed)
/// 2. First harness found on PATH (in CANDIDATES order)
/// 3. Error with install instructions if nothing is found
pub fn resolve_harness(config_dir: &Path) -> Result<String> {
    let installed = detect_installed();

    if installed.is_empty() {
        bail!(
            "No AI coding harness found on PATH.\n\
             \n\
             Install one of the following and make sure it is on your PATH:\n\
             \n\
               • opencode (OpenCode)    — https://opencode.ai\n\
               • claude  (Claude Code)  — https://claude.ai/code\n\
               • codex   (OpenAI Codex) — https://github.com/openai/codex\n\
             \n\
             After installing, verify with:  which opencode  (or claude / codex)"
        );
    }

    // Honour explicit user preference from config
    if let Some(preferred) = get_default_harness(config_dir) {
        if installed
            .iter()
            .any(|h| h.name == preferred || h.binary == preferred)
        {
            return Ok(preferred);
        }
        // Preferred harness is configured but not installed — warn and fall through.
        // Include the install URL so the user knows exactly how to fix it.
        let url = install_url(&preferred);
        eprintln!(
            "WARNING: configured default_harness '{preferred}' is not installed or not on PATH.\n\
             Install it from: {url}\n\
             Falling back to first available harness: {first}.",
            first = installed[0].binary
        );
    }

    // Single harness: auto-select without prompting
    if installed.len() == 1 {
        return Ok(installed[0].binary.clone());
    }

    // Multiple installed and no preference: use first in discovery order
    Ok(installed[0].binary.clone())
}

/// Build the complete shell command string for the selected harness.
///
/// When `interactive` is true (orchestrator), the harness stays alive for
/// follow-up messages sent via tmux send-keys.  When false (workers), the
/// harness processes the prompt once and exits.
///
/// CLI conventions per harness:
///
/// Interactive (orchestrator):
///   - opencode: `opencode [--model <m>] --prompt <task>`
///   - claude:   `claude [--model <m>] <task>`
///   - codex:    `codex [--model <m>] <task>`
///
/// One-shot (workers):
///   - opencode: `opencode [--model <m>] --prompt <task>`
///   - claude:   `claude -p [--model <m>] <task>`
///   - codex:    `codex exec [--model <m>] <task>`
pub fn build_harness_cmd(
    harness: &str,
    model: Option<&str>,
    prompt: &str,
    interactive: bool,
) -> String {
    let escaped = shell_escape(prompt);

    match harness {
        "claude" => {
            // Skip OpenRouter-format models (contain '/') — Claude Code uses
            // its own native model names (e.g. "claude-sonnet-4-6").
            let native_model = model.filter(|m| !m.contains('/'));
            let model_flag = model_flag(native_model);
            if interactive {
                // Interactive: stays alive, receives heartbeats via tmux send-keys.
                format!("claude{model_flag} {escaped}")
            } else {
                // One-shot: -p (print mode) processes prompt and exits.
                format!("claude -p{model_flag} {escaped}")
            }
        }
        "codex" => {
            // Skip OpenRouter-format models (contain '/') — Codex uses its own
            // native model names (e.g. "o3", "gpt-4o").
            let native_model = model.filter(|m| !m.contains('/'));
            let model_flag = model_flag(native_model);
            if interactive {
                // Interactive TUI: stays alive, busy-state detected via TUI patterns.
                format!("codex{model_flag} {escaped}")
            } else {
                // Non-interactive: exec processes prompt and exits.
                format!("codex exec{model_flag} {escaped}")
            }
        }
        // Default (opencode or any unknown harness): opencode-style interface.
        // opencode --prompt pre-fills and submits but stays interactive.
        _ => {
            let model_flag = model_flag(model);
            format!("opencode{model_flag} --prompt {escaped}")
        }
    }
}

/// Format the `--model` flag string (with a leading space), or empty string if None.
fn model_flag(model: Option<&str>) -> String {
    match model {
        Some(m) => format!(" --model {}", shell_escape(m)),
        None => String::new(),
    }
}

/// Validate a harness name against known candidates, returning an error for unknown names.
/// Also warns on stderr if the harness is not actually installed on PATH.
pub fn validate_harness_name(name: &str) -> Result<()> {
    let known = ["opencode", "claude", "codex"];
    if !known.contains(&name) {
        bail!(
            "Unknown harness {:?}. Valid options: opencode, claude, codex",
            name
        );
    }
    let installed = detect_installed();
    if !installed.iter().any(|h| h.name == name || h.binary == name) {
        eprintln!(
            "WARNING: '{}' does not appear to be installed on PATH.\n\
             Install it from: {}",
            name,
            install_url(name)
        );
    }
    Ok(())
}

/// Return a short, install-guide URL for a harness name.
pub fn install_url(name: &str) -> &'static str {
    match name {
        "opencode" => "https://opencode.ai",
        "claude" => "https://claude.ai/code",
        "codex" => "https://github.com/openai/codex",
        _ => "https://opencode.ai",
    }
}

/// Read `default_model` from `<config_dir>/config.json`.
/// Returns `None` if the file does not exist or the field is absent.
pub fn get_default_model(config_dir: &Path) -> Option<String> {
    let config_path = config_dir.join("config.json");
    if !config_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&config_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v["default_model"].as_str().map(String::from)
}

/// Read the model for a specific harness from `<config_dir>/config.json`.
///
/// Priority:
///   1. `models.<harness>` — per-harness override
///   2. `default_model` — global fallback
///   3. `None` — let the harness use its own default
pub fn get_model_for_harness(config_dir: &Path, harness: &str) -> Option<String> {
    let config_path = config_dir.join("config.json");
    if !config_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&config_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Check per-harness override first
    if let Some(m) = v["models"][harness].as_str() {
        if !m.is_empty() {
            return Some(m.to_string());
        }
    }

    // Fall back to global default_model
    v["default_model"].as_str().map(String::from)
}

/// Show an interactive arrow-key harness picker in the current terminal.
///
/// Displays all `harnesses` (installed or not), highlights the one matching
/// `current` (if any), and lets the user navigate with ↑/↓ and confirm with
/// Enter. Returns `Some(name)` on selection or `None` when the user cancels
/// with `q` / `Esc`.
///
/// This function temporarily enables raw mode and hides the cursor.  Both are
/// unconditionally restored before returning, even on error.
pub fn run_interactive_picker(
    harnesses: &[HarnessInfo],
    current: Option<&str>,
) -> Result<Option<String>> {
    use crossterm::{
        cursor,
        event::{self, Event, KeyCode, KeyModifiers},
        execute, queue,
        style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
        terminal::{self, Clear, ClearType},
    };
    use std::io::{self, Write};

    if harnesses.is_empty() {
        return Ok(None);
    }

    let installed = detect_installed();
    let installed_set: std::collections::HashSet<String> =
        installed.iter().map(|h| h.name.clone()).collect();

    // Initial selection: prefer the currently configured harness, else first.
    let initial_idx = current
        .and_then(|c| harnesses.iter().position(|h| h.name == c || h.binary == c))
        .unwrap_or(0);
    let mut selected = initial_idx;

    let mut stdout = io::stdout();

    // Enable raw mode so we can receive individual key events.
    terminal::enable_raw_mode()?;
    let _ = execute!(stdout, cursor::Hide);

    // We draw (3 header + len list + 2 footer) lines each render cycle.
    // Saving this so we can move the cursor back to the top on re-draw.
    let list_len = harnesses.len() as u16;
    let render_lines: u16 = 3 + list_len + 2; // blank + title + blank + items + blank + hints

    let mut first_draw = true;

    let result = 'picker: loop {
        // On re-draw, move cursor back to the start of the block.
        if !first_draw {
            let _ = execute!(
                stdout,
                cursor::MoveUp(render_lines),
                cursor::MoveToColumn(0)
            );
        }
        first_draw = false;

        // Header (3 lines)
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            Print("\r\n"),
            Clear(ClearType::CurrentLine),
            SetAttribute(Attribute::Bold),
            Print("  Select default AI harness:\r\n"),
            SetAttribute(Attribute::Reset),
            Clear(ClearType::CurrentLine),
            Print("\r\n"),
        )?;

        // Harness list (inlined to avoid dyn Write issues with queue! macro)
        for (i, h) in harnesses.iter().enumerate() {
            let inst = installed_set.contains(&h.name);
            let status = if inst { "[installed]" } else { "[not found]" };
            queue!(stdout, Clear(ClearType::CurrentLine))?;
            if i == selected {
                queue!(
                    stdout,
                    SetForegroundColor(Color::Green),
                    SetAttribute(Attribute::Bold),
                    Print(format!(
                        "  \u{25ba} {:<10} {:<20}  {}\r\n",
                        h.binary, h.display_name, status
                    )),
                    SetAttribute(Attribute::Reset),
                    ResetColor,
                )?;
            } else if inst {
                queue!(
                    stdout,
                    SetForegroundColor(Color::Reset),
                    Print(format!(
                        "    {:<10} {:<20}  {}\r\n",
                        h.binary, h.display_name, status
                    )),
                    ResetColor,
                )?;
            } else {
                queue!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!(
                        "    {:<10} {:<20}  {}\r\n",
                        h.binary, h.display_name, status
                    )),
                    ResetColor,
                )?;
            }
        }

        // Footer (2 lines)
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            Print("\r\n"),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::DarkGrey),
            Print("  \u{2191}\u{2193} move  Enter select  q cancel\r\n"),
            ResetColor,
        )?;
        stdout.flush()?;

        // Key event loop: spin until we get an actionable key.
        loop {
            if event::poll(std::time::Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            if selected > 0 {
                                selected -= 1;
                            }
                            break; // re-render outer loop
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if selected + 1 < harnesses.len() {
                                selected += 1;
                            }
                            break; // re-render outer loop
                        }
                        KeyCode::Enter => {
                            break 'picker Some(harnesses[selected].name.clone());
                        }
                        KeyCode::Char('q') | KeyCode::Esc => {
                            break 'picker None;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break 'picker None;
                        }
                        _ => {}
                    }
                }
            }
        }
    };

    // Restore terminal state unconditionally.
    let _ = terminal::disable_raw_mode();
    let _ = execute!(stdout, cursor::Show);
    let _ = writeln!(stdout);

    Ok(result)
}
