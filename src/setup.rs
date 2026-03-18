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

    let preferences = build_preferences_section(&user_cfg);
    // Note: replace $HARNESS_DISPLAY before $HARNESS so the longer token is matched first.
    let content = AGENTS_MD
        .replace("$BIN", bin)
        .replace("$PREFERENCES", &preferences)
        .replace("$HARNESS_DISPLAY", harness_display)
        .replace("$HARNESS", &harness_name)
        .replace("$DEFAULT_MODEL", &default_model);

    let agents_path = base.join("AGENTS.md");
    if agents_path.exists() {
        let existing = fs::read_to_string(&agents_path)?;
        let updated = merge_agents_content(&existing, &content);
        fs::write(&agents_path, &updated).context("failed to update AGENTS.md")?;
        eprintln!("updated {}", agents_path.display());
    } else {
        fs::write(&agents_path, &content).context("failed to write AGENTS.md")?;
        eprintln!("wrote {}", agents_path.display());
    }

    Ok(())
}

/// Merge a new superharness section into existing AGENTS.md content.
///
/// Three cases:
/// 1. Existing file already has `# SuperHarness` → replace that section in-place.
/// 2. Existing file has custom content but no superharness section → append with
///    a `<!-- SUPERHARNESS INSTRUCTIONS BELOW -->` marker so the orchestrator can
///    interactively merge on the next launch.
/// 3. Existing file is empty → write the new section directly.
pub fn merge_agents_content(existing: &str, new_section: &str) -> String {
    if existing.contains("# SuperHarness") {
        // Strip old superharness section and replace with the updated one.
        let before = existing
            .split("# SuperHarness")
            .next()
            .unwrap_or("")
            .trim_end();
        if before.is_empty() {
            new_section.to_string()
        } else {
            format!("{before}\n\n{new_section}")
        }
    } else if existing.trim().is_empty() {
        new_section.to_string()
    } else {
        // Custom content, no superharness section yet — append with marker.
        format!("{existing}\n<!-- SUPERHARNESS INSTRUCTIONS BELOW -->\n{new_section}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_SECTION: &str = "# SuperHarness\n\n> You are superharness.\n\n$TASK\n";

    // ── merge_agents_content ────────────────────────────────────────────────

    #[test]
    fn merge_fresh_file_returns_new_section() {
        // No existing file content at all
        let result = merge_agents_content("", FAKE_SECTION);
        assert_eq!(result, FAKE_SECTION);
    }

    #[test]
    fn merge_whitespace_only_returns_new_section() {
        let result = merge_agents_content("   \n  \n", FAKE_SECTION);
        assert_eq!(result, FAKE_SECTION);
    }

    #[test]
    fn merge_existing_superharness_only_replaces_section() {
        // File is purely a superharness section (no custom content before it)
        let existing = "# SuperHarness\n\n> Old content.\n\n$TASK\n";
        let result = merge_agents_content(existing, FAKE_SECTION);
        assert_eq!(
            result, FAKE_SECTION,
            "should replace the old section exactly"
        );
        assert!(!result.contains("Old content"), "old content must be gone");
    }

    #[test]
    fn merge_existing_superharness_preserves_custom_prefix() {
        let existing = "# My Project\n\nSome docs.\n\n# SuperHarness\n\n> Old content.\n";
        let result = merge_agents_content(existing, FAKE_SECTION);
        assert!(
            result.starts_with("# My Project"),
            "custom prefix must be kept"
        );
        assert!(result.contains(FAKE_SECTION), "new section must be present");
        assert!(
            !result.contains("Old content"),
            "old superharness content must be gone"
        );
        // Should not duplicate the section
        assert_eq!(
            result.matches("# SuperHarness").count(),
            1,
            "only one superharness section"
        );
    }

    #[test]
    fn merge_custom_content_no_superharness_appends_with_marker() {
        let existing = "# My Project\n\nCustom instructions.\n";
        let result = merge_agents_content(existing, FAKE_SECTION);
        assert!(
            result.contains("# My Project"),
            "original content preserved"
        );
        assert!(
            result.contains("<!-- SUPERHARNESS INSTRUCTIONS BELOW -->"),
            "marker must be present"
        );
        assert!(result.contains(FAKE_SECTION), "new section appended");
        // Marker comes before the superharness section
        let marker_pos = result
            .find("<!-- SUPERHARNESS INSTRUCTIONS BELOW -->")
            .unwrap();
        let section_pos = result.find("# SuperHarness").unwrap();
        assert!(marker_pos < section_pos, "marker must precede the section");
    }

    #[test]
    fn merge_idempotent_on_second_run() {
        // Simulate: first run on a fresh project → write template.
        // Second run → template is replaced in-place (no duplication).
        let first = merge_agents_content("", FAKE_SECTION);
        let second = merge_agents_content(&first, FAKE_SECTION);
        assert_eq!(
            second.matches("# SuperHarness").count(),
            1,
            "re-running must not duplicate the section"
        );
        assert_eq!(
            second, FAKE_SECTION,
            "content should be identical to a fresh write"
        );
    }

    #[test]
    fn merge_custom_content_idempotent_after_marker_and_section_written() {
        // After the first append (with marker), a second run should recognise
        // `# SuperHarness` in the file and replace — not append again.
        let existing = "# My Project\n\nCustom instructions.\n";
        let after_first = merge_agents_content(existing, FAKE_SECTION);
        let after_second = merge_agents_content(&after_first, FAKE_SECTION);
        assert_eq!(
            after_second.matches("# SuperHarness").count(),
            1,
            "second run must not create a second superharness section"
        );
    }
}
