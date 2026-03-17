use crate::{events, heartbeat, output_cleaner, tmux};
use anyhow::Result;

/// Handle `Command::List`.
pub fn handle_list() -> Result<()> {
    let panes = tmux::list()?;
    let out = serde_json::json!({ "panes": panes });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Read`.
pub fn handle_read(pane: String, lines: u32, raw: bool) -> Result<()> {
    let captured = tmux::read(&pane, lines)?;
    let output = if raw {
        captured
    } else {
        output_cleaner::clean_output(&captured)
    };
    let out = serde_json::json!({ "pane": pane, "output": output });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Send`.
pub fn handle_send(pane: String, text: String) -> Result<()> {
    tmux::send(&pane, &text)?;
    let out = serde_json::json!({ "pane": pane, "sent": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Kill`.
pub fn handle_kill(pane: String) -> Result<()> {
    tmux::kill(&pane)?;
    let _ = events::log_event(
        events::EventKind::WorkerKilled,
        Some(&pane),
        "worker killed",
    );
    // Trigger a heartbeat so the orchestrator wakes up immediately.
    let _ = heartbeat::heartbeat();
    let out = serde_json::json!({ "pane": pane, "killed": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Hide`.
pub fn handle_hide(pane: String, name: Option<String>) -> Result<()> {
    tmux::hide(&pane, name.as_deref())?;
    let out = serde_json::json!({ "pane": pane, "hidden": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Show`.
pub fn handle_show(pane: String, split: String) -> Result<()> {
    tmux::show(&pane, &split)?;
    let out = serde_json::json!({ "pane": pane, "visible": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Surface`.
pub fn handle_surface(pane: String) -> Result<()> {
    tmux::surface(&pane)?;
    let out = serde_json::json!({ "pane": pane, "visible": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Compact`.
pub fn handle_compact() -> Result<()> {
    let (moved, remaining) = tmux::compact_panes()?;
    let note = if moved > 0 {
        format!("{moved} agent(s) moved to background tabs. {remaining} agent(s) remain visible.")
    } else {
        "No agents needed moving — all agents meet size thresholds.".to_string()
    };
    let out = serde_json::json!({
        "moved_to_background": moved,
        "still_visible": remaining,
        "note": note,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Resize`.
pub fn handle_resize(pane: String, direction: String, amount: u32) -> Result<()> {
    tmux::resize(&pane, &direction, amount)?;
    let out = serde_json::json!({ "pane": pane, "resized": true });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::Layout`.
pub fn handle_layout(name: String) -> Result<()> {
    tmux::layout(&name)?;
    let out = serde_json::json!({ "layout": name });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Handle `Command::SmartLayout`.
pub fn handle_smart_layout(hint: Option<String>) -> Result<()> {
    let action = match hint.as_deref() {
        // "maximize <pane_id>" — give that pane extra space and surface it
        Some(h) if h.starts_with("maximize ") => {
            let pane_id = h["maximize ".len()..].trim();
            tmux::smart_layout_with_attention(Some(pane_id))?;
            format!("maximized {pane_id}")
        }
        // "focus <pane_id>" — surface then rebalance
        Some(h) if h.starts_with("focus ") => {
            let pane_id = h["focus ".len()..].trim();
            tmux::surface(pane_id)?;
            tmux::smart_layout()?;
            format!("focused {pane_id}")
        }
        // "rebalance" or no hint — standard smart layout
        _ => {
            tmux::smart_layout()?;
            "rebalanced".to_string()
        }
    };
    let out = serde_json::json!({ "layout": "smart", "action": action, "hint": hint });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
