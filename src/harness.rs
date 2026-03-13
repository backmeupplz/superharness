//! Harness detection and selection.
//!
//! Supported harnesses and their CLI interfaces:
//!   - opencode:  opencode [--model <m>] --prompt <task>
//!   - claude:    claude [--model <m>] --print <task>   (Claude Code, non-interactive)
//!   - codex:     codex [--model <m>] <task>            (positional argument)

use anyhow::{bail, Result};
use std::path::Path;

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
             Install one of the following:\n\
             \n\
             • opencode (OpenCode)    — https://opencode.ai\n\
             • claude  (Claude Code)  — https://claude.ai/code\n\
             • codex   (OpenAI Codex) — https://github.com/openai/codex"
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
        // Preferred harness is configured but not installed — warn and fall through
        eprintln!(
            "WARNING: configured default_harness '{preferred}' is not installed. \
             Falling back to first available harness."
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
/// CLI conventions per harness:
///   - opencode: `opencode [--model <m>] --prompt <task>`
///   - claude:   `claude [--model <m>] --print <task>`
///   - codex:    `codex [--model <m>] <task>`
pub fn build_harness_cmd(harness: &str, model: Option<&str>, prompt: &str) -> String {
    let escaped = shell_escape(prompt);

    match harness {
        "claude" => {
            let model_flag = model_flag(model);
            format!("claude{model_flag} --print {escaped}")
        }
        "codex" => {
            let model_flag = model_flag(model);
            format!("codex{model_flag} {escaped}")
        }
        // Default (opencode or any unknown harness): opencode-style interface
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

/// Shell-escape a string by single-quoting it.
pub fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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
