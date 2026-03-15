use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::harness;
use crate::util;

/// User-level model/provider preferences. Lives at ~/.config/superharness/config.json.
/// All fields are optional — missing file or missing fields are silently ignored.
///
/// Example config:
/// ```json
/// {
///   "default_model": "anthropic/claude-sonnet-4-6",
///   "preferred_providers": ["anthropic"],
///   "preferred_models": [
///     "anthropic/claude-sonnet-4-6",
///     "anthropic/claude-opus-4-5"
///   ]
/// }
/// ```
#[derive(Debug, Deserialize, Default)]
pub struct ProviderRouting {
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UserConfig {
    pub default_model: Option<String>,
    pub preferred_providers: Option<Vec<String>>,
    pub preferred_models: Option<Vec<String>>,
    pub provider_routing: Option<ProviderRouting>,
}

pub fn load_user_config() -> UserConfig {
    let path = util::superharness_config_dir().join("config.json");

    if !path.exists() {
        return UserConfig::default();
    }

    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => UserConfig::default(),
    }
}

fn build_preferences_section(cfg: &UserConfig) -> String {
    let has_default = cfg.default_model.is_some();
    let has_providers = cfg
        .preferred_providers
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let has_models = cfg
        .preferred_models
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let has_routing = cfg
        .provider_routing
        .as_ref()
        .and_then(|r| r.note.as_ref())
        .map(|n| !n.is_empty())
        .unwrap_or(false);

    if !has_default && !has_providers && !has_models && !has_routing {
        return String::new();
    }

    let mut out = String::from("## Model Preferences\n\n");
    out.push_str(
        "The user has configured model preferences. Follow these when spawning workers \
        unless the task genuinely requires something different (e.g. a vision-specific model).\n\n",
    );

    if let Some(ref m) = cfg.default_model {
        out.push_str(&format!("**Default model:** `{m}`\n\n"));
    }

    if has_routing {
        let note = cfg
            .provider_routing
            .as_ref()
            .unwrap()
            .note
            .as_ref()
            .unwrap();
        out.push_str(&format!("**Provider routing rule:** {note}\n\n"));
    }

    if has_providers {
        let providers = cfg.preferred_providers.as_ref().unwrap();
        out.push_str("**Preferred providers** (prefer these over others for equivalent models):\n");
        for p in providers {
            out.push_str(&format!("- {p}\n"));
        }
        out.push('\n');
    }

    if has_models {
        let models = cfg.preferred_models.as_ref().unwrap();
        out.push_str("**Preferred models** (use these by default):\n");
        for m in models {
            out.push_str(&format!("- `{m}`\n"));
        }
        out.push('\n');
    }

    out
}

const AGENTS_MD: &str = include_str!("../assets/agents_template.md");

/// Map a harness name to its human-readable display name.
fn harness_display_name(harness: &str) -> &'static str {
    match harness {
        "claude" => "Claude Code",
        "codex" => "OpenAI Codex",
        _ => "OpenCode",
    }
}

fn get_available_models(harness: &str) -> String {
    let output = std::process::Command::new(harness).arg("models").output();

    match output {
        Ok(o) if o.status.success() => {
            let models = String::from_utf8_lossy(&o.stdout);
            let lines: Vec<&str> = models.lines().filter(|l| !l.is_empty()).collect();
            if lines.is_empty() {
                return format!(
                    "(could not detect models — run `{harness} models` to see available)"
                );
            }
            lines.join("\n")
        }
        _ => format!("(could not detect models — run `{harness} models` to see available)"),
    }
}

fn get_authenticated_providers(harness: &str) -> String {
    let output = std::process::Command::new(harness)
        .args(["auth", "list"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            // Strip ANSI codes
            let stripped: String = text
                .replace("\x1b[90m", "")
                .replace("\x1b[0m", "")
                .replace("│", "|")
                .replace("●", "-")
                .replace("┌", "")
                .replace("└", "")
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            stripped
        }
        _ => format!("(could not detect — run `{harness} auth list`)"),
    }
}

pub fn write_config(dir: &str, bin: &str) -> Result<()> {
    let base = Path::new(dir);
    let config_dir = util::superharness_config_dir();

    // Resolve the active harness (falls back to first installed if no preference set).
    let harness_name =
        harness::resolve_harness(&config_dir).unwrap_or_else(|_| "opencode".to_string());
    let harness_display = harness_display_name(&harness_name);

    let user_cfg = load_user_config();

    // Resolve the default model: user config → harness config → sensible fallback.
    let default_model = user_cfg
        .default_model
        .clone()
        .or_else(|| harness::get_default_model(&config_dir))
        .unwrap_or_else(|| "anthropic/claude-sonnet-4-6".to_string());

    let models = get_available_models(&harness_name);
    let providers = get_authenticated_providers(&harness_name);
    let preferences = build_preferences_section(&user_cfg);
    // Note: replace $HARNESS_DISPLAY before $HARNESS so the longer token is matched first.
    let content = AGENTS_MD
        .replace("$BIN", bin)
        .replace("$MODELS", &models)
        .replace("$PROVIDERS", &providers)
        .replace("$PREFERENCES", &preferences)
        .replace("$HARNESS_DISPLAY", harness_display)
        .replace("$HARNESS", &harness_name)
        .replace("$DEFAULT_MODEL", &default_model);

    let agents_path = base.join("AGENTS.md");
    if agents_path.exists() {
        let existing = fs::read_to_string(&agents_path)?;
        if existing.contains("SuperHarness Orchestrator") {
            // Strip existing superharness section and rewrite with current binary path
            let before = existing
                .split("# SuperHarness Orchestrator")
                .next()
                .unwrap_or("")
                .trim_end();
            if before.is_empty() {
                fs::write(&agents_path, &content).context("failed to write AGENTS.md")?;
            } else {
                let combined = format!("{before}\n\n{content}");
                fs::write(&agents_path, combined).context("failed to update AGENTS.md")?;
            }
            eprintln!("updated {}", agents_path.display());
        } else {
            let combined = format!("{existing}\n{content}");
            fs::write(&agents_path, combined).context("failed to update AGENTS.md")?;
            eprintln!("updated {}", agents_path.display());
        }
    } else {
        fs::write(&agents_path, &content).context("failed to write AGENTS.md")?;
        eprintln!("wrote {}", agents_path.display());
    }

    Ok(())
}
