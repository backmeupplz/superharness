use anyhow::{bail, Context, Result};
use std::process::Command;

use crate::harness;
use crate::heartbeat;
use crate::util;

use super::{set_orchestrator_pane_id, tmux, tmux_ok, SESSION};

/// Detect whether we are running inside an existing tmux session.
/// Returns `true` when the `$TMUX` environment variable is set and non-empty.
fn inside_tmux() -> bool {
    std::env::var("TMUX")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// When running inside tmux, query the current session name so we can switch
/// back to it on exit.  Returns `None` outside tmux or on any error.
fn current_session_name() -> Option<String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}"])
        .output()
        .ok()?;
    if output.status.success() {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    } else {
        None
    }
}

pub(super) fn has_session() -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", SESSION])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(super) fn ensure_session() -> Result<()> {
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
    tmux_ok(&["set-option", "-t", SESSION, "status-interval", "1"])?;
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
    // Clicking anywhere on the right side of the status bar toggles the heartbeat on/off.
    let _ = tmux_ok(&[
        "bind-key",
        "-n",
        "MouseDown1StatusRight",
        "run-shell",
        &format!("{bin_path} heartbeat-toggle"),
    ]);

    // Right side: dynamic shell fragments read mode + pane count.
    // Uses grep to extract mode from the project-local .superharness/state.json.
    // Falls back to the global active_project.txt to locate the project dir.
    // The shell snippet produces "AWAY" or "PRESENT" from the state file.
    let mode_snippet = r##"#(p=$(cat $HOME/.local/share/superharness/active_project.txt 2>/dev/null); f="$p/.superharness/state.json"; if [ -f "$f" ]; then m=$(jq -r '.mode' "$f" 2>/dev/null | tr '[:lower:]' '[:upper:]'); [ -z "$m" ] && m=$(grep -o '"mode"[[:space:]]*:[[:space:]]*"[^"]*"' "$f" | grep -o '"[^"]*"$' | tr -d '"' | tr '[:lower:]' '[:upper:]'); [ "$m" = "AWAY" ] && echo "#[fg=colour214,bold]AWAY#[default]" || echo "#[fg=colour71,bold]PRESENT#[default]"; else echo "#[fg=colour71,bold]PRESENT#[default]"; fi)"##;

    // Heartbeat indicator: shows icon + seconds to next beat.
    // Uses ● (U+25CF filled circle) which is single-width in all terminals.
    let heartbeat_snippet = format!(
        "#({bin_path} heartbeat-status 2>/dev/null || echo '#[fg=colour245](^_^) --#[default]')"
    );

    // Worker count for F4 button label: total worker pane count.
    let worker_count_snippet = format!("#({bin_path} status-counts 2>/dev/null || echo '0')");

    let status_right = format!(
        "#[fg=colour240]│ #[fg=colour214]MODE:{mode_snippet} \
         #[fg=colour240]│ {heartbeat_snippet} \
         #[fg=colour240]│ #[fg=colour110] F1:toggle-away #[fg=colour240] │ #[fg=colour110] F2:settings #[fg=colour240] │ #[fg=colour110] F3:status #[fg=colour240] │ #[fg=colour110] F4:workers ({worker_count_snippet}) #[fg=colour240] │ #[fg=colour110] F5:tasks #[fg=colour240] │ #[fg=colour110] F6:events  #[default]"
    );

    tmux_ok(&["set-option", "-t", SESSION, "status-right", &status_right])?;
    tmux_ok(&["set-option", "-t", SESSION, "status-right-length", "180"])?;

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
    let config_dir_base = util::superharness_config_dir();
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

    // Detect whether an existing AGENTS.md had custom content and the superharness section
    // was appended (rather than this being a clean write).  The marker is written by setup.rs
    // when it encounters an AGENTS.md that doesn't already contain a superharness section.
    let agents_path = std::path::PathBuf::from(&dir_str).join("AGENTS.md");
    let agents_merge_note: String = if agents_path.exists() {
        let agents_content = std::fs::read_to_string(&agents_path).unwrap_or_default();
        if agents_content.contains("<!-- SUPERHARNESS INSTRUCTIONS BELOW -->") {
            " NOTE: I found an existing AGENTS.md with custom project content. \
            The superharness instructions have been appended at the bottom of that file \
            (below the '<!-- SUPERHARNESS INSTRUCTIONS BELOW -->' marker). \
            Please read AGENTS.md right now, then integrate the superharness instructions \
            with the existing custom content so they work together coherently — \
            restructure, merge, or annotate sections as needed, and remove the \
            HTML marker comment once the merge is done. \
            After integrating, tell the user what you changed."
                .to_string()
        } else {
            String::new()
        }
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
            confirm it's done and ask what they'd like to work on today.{agents_merge_note}"
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
                Be conversational — this is a planning chat, not a form to fill out.{agents_merge_note}"
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
                Please acknowledge this state and continue from where you left off, or ask the user what they want to work on.{agents_merge_note}",
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

    // ── Detect whether we are inside an existing tmux session ──────────────
    let embedded = inside_tmux();
    let original_session = if embedded {
        current_session_name()
    } else {
        None
    };

    tmux_ok(&["new-session", "-d", "-s", SESSION, "-c", &dir_str])?;
    tmux_ok(&["rename-window", "-t", SESSION, "superharness"])?;

    // When embedded, the new session's first pane may get a different ID than %0.
    // Query the actual pane ID of the orchestrator pane we just created.
    let orch_pane = tmux(&[
        "list-panes",
        "-t",
        &format!("{SESSION}:0"),
        "-F",
        "#{pane_id}",
    ])
    .unwrap_or_else(|_| "%0".to_string())
    .lines()
    .next()
    .unwrap_or("%0")
    .to_string();

    // Store the orchestrator pane ID so all subcommands can find it.
    set_orchestrator_pane_id(&orch_pane)?;

    tmux_ok(&["select-pane", "-t", &orch_pane, "-T", "superharness"])?;
    configure_session(bin_path)?;
    export_env_to_session()?;

    // ── Heartbeat background thread ─────────────────────────────────────────
    // Spawn an in-process background thread instead of a separate tmux window.
    // The thread manages countdown, trigger files, and state persistence.
    heartbeat::start_thread();

    // Replace default shell with splash+opencode.
    // Use bash -lc (login shell) so that ~/.profile and ~/.bash_profile are
    // sourced, ensuring PATH and credential env vars are fully initialised.
    tmux_ok(&["respawn-pane", "-t", SESSION, "-k", "bash", "-lc", &splash])?;

    if embedded {
        // ── Embedded mode (inside existing tmux) ────────────────────────────
        // Switch the current tmux client to the superharness session instead
        // of trying to attach (which would create a broken nested session).
        let status = Command::new("tmux")
            .args(["switch-client", "-t", SESSION])
            .status()
            .context("failed to switch tmux client to superharness session")?;

        if !status.success() {
            // Cleanup on switch failure
            if has_session() {
                let _ = tmux_ok(&["kill-session", "-t", SESSION]);
            }
            bail!("tmux switch-client failed");
        }

        // Block until the superharness session is gone (the user switched away
        // or the orchestrator exited). Poll every second.
        loop {
            if !has_session() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        // Switch back to the original session if it still exists.
        if let Some(ref orig) = original_session {
            let _ = Command::new("tmux")
                .args(["switch-client", "-t", orig])
                .status();
        }
    } else {
        // ── Standalone mode (not inside tmux) ───────────────────────────────
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
    }

    Ok(())
}
