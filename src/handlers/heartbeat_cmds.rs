use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::heartbeat;
use crate::project;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Handle `Command::Heartbeat`.
///
/// Two modes:
///
/// **No args** (`superharness heartbeat`, called by workers):
///   Write a trigger file so the background thread fires a beat immediately.
///
/// **`--snooze N`** (`superharness heartbeat --snooze N`, called by orchestrator):
///   Write N to the snooze file so the background thread adds N to its countdown.
pub fn handle_heartbeat(snooze: Option<u64>) -> Result<()> {
    let state_dir = project::get_project_state_dir()?;

    if let Some(secs) = snooze {
        // Snooze: write N to the snooze file for the background thread to pick up.
        let snooze_path = state_dir.join("heartbeat_snooze");
        std::fs::write(&snooze_path, secs.to_string())?;
        eprintln!("[heartbeat] snooze {secs}s requested via file");
    } else {
        // Worker-triggered immediate beat: write trigger file.
        let trigger_path = state_dir.join("heartbeat_trigger");
        std::fs::write(&trigger_path, "1")?;
        eprintln!("[heartbeat] trigger file written — background thread will fire beat");
    }

    Ok(())
}

/// Handle `Command::HeartbeatToggle`.
///
/// Write a toggle trigger file for the background thread to pick up.
pub fn handle_heartbeat_toggle() -> Result<()> {
    let state_dir = project::get_project_state_dir()?;
    let toggle_path = state_dir.join("heartbeat_toggle_trigger");
    std::fs::write(&toggle_path, "1")?;
    eprintln!("[heartbeat] toggle trigger file written");
    Ok(())
}

/// Handle `Command::HeartbeatStatus` — print heartbeat status for tmux status bar.
///
/// Pure display: reads state file, prints kaomoji + countdown. Never fires anything.
///
/// Simplified faces:
/// - Disabled: `(x_x)`
/// - No scheduled beat (`next_beat_ts == 0`): `(^_^) --`
/// - Normal countdown: `(^_^) Ns`
/// - Just fired (within 3s): `(^o^) Ns`
pub fn handle_heartbeat_status() -> Result<()> {
    let now = now_secs();

    let state = heartbeat::read_heartbeat_state();

    // Permanently disabled.
    if state.disabled {
        print!("#[fg=colour240](x_x)#[default]");
        return Ok(());
    }

    // No scheduled beat (uninitialized or cleared).
    if state.next_beat_ts == 0 {
        print!("#[fg=colour245](^_^) --#[default]");
        return Ok(());
    }

    let secs_to_next = state.next_beat_ts.saturating_sub(now);

    if state.last_beat_ts > 0 && now.saturating_sub(state.last_beat_ts) <= 3 {
        // Just fired — excited, bright green.
        print!("#[fg=colour156](^o^) {secs_to_next}s#[default]");
    } else {
        // Normal — happy, calm green.
        print!("#[fg=colour114](^_^) {secs_to_next}s#[default]");
    }

    Ok(())
}
