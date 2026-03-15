use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::heartbeat;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Handle `Command::HeartbeatDaemonTick`.
///
/// Called every 1s by the hidden daemon loop:
///   `while true; do superharness heartbeat-daemon-tick 2>/dev/null; sleep 1; done`
///
/// Silent — no stdout output. All logic lives in `heartbeat::daemon_tick()`.
pub fn handle_heartbeat_daemon_tick() -> Result<()> {
    heartbeat::daemon_tick()
}

/// Handle `Command::Heartbeat`.
///
/// Two modes:
///
/// **No args** (`superharness heartbeat`, called by workers):
///   Send [HEARTBEAT] to %0 immediately, bypassing all countdown/busy checks.
///   Reset `next_beat_ts` to `now + interval` so the daemon's countdown restarts.
///
/// **`--snooze N`** (`superharness heartbeat --snooze N`, called by orchestrator):
///   Add N seconds to `next_beat_ts` (additive snooze). No heartbeat is sent.
///   The next beat after the snoozed one returns to the normal interval.
pub fn handle_heartbeat(snooze: Option<u64>) -> Result<()> {
    let now = now_secs();

    if let Some(secs) = snooze {
        // Snooze: shift the next beat forward by N seconds (additive).
        let mut state = heartbeat::read_heartbeat_state();
        state.next_beat_ts = state.next_beat_ts.saturating_add(secs);
        heartbeat::write_heartbeat_state(&state);
        eprintln!(
            "[heartbeat] snoozed {secs}s — next beat at unix {}",
            state.next_beat_ts
        );
    } else {
        // Worker-triggered immediate beat: bypass all checks and send now.
        let needs_attention = match heartbeat::heartbeat() {
            Ok(na) => {
                eprintln!("[heartbeat] sent [HEARTBEAT] to %0");
                na
            }
            Err(e) => {
                eprintln!("[heartbeat] error: {e}");
                false
            }
        };

        // Reset the daemon countdown so it doesn't fire again immediately.
        let mut state = heartbeat::read_heartbeat_state();
        let interval = if state.interval_secs == 0 {
            heartbeat::get_interval()
        } else {
            state.interval_secs
        };
        state.last_beat_ts = now;
        state.next_beat_ts = now + interval;
        state.last_sent = true;
        state.needs_attention = needs_attention;
        heartbeat::write_heartbeat_state(&state);
    }

    Ok(())
}

/// Handle `Command::HeartbeatToggle`.
///
/// Flips the `disabled` flag.
/// When toggling **on**: also resets `next_beat_ts = now + interval` so the
/// countdown starts fresh instead of immediately re-firing a stale beat.
pub fn handle_heartbeat_toggle() -> Result<()> {
    let now = now_secs();
    let mut state = heartbeat::read_heartbeat_state();

    if state.disabled {
        // Currently disabled — re-enable and start a fresh countdown.
        let interval = if state.interval_secs == 0 {
            heartbeat::get_interval()
        } else {
            state.interval_secs
        };
        state.disabled = false;
        state.next_beat_ts = now + interval;
        heartbeat::write_heartbeat_state(&state);
        eprintln!("[heartbeat] toggled on (resumed)");
    } else {
        // Currently enabled — disable permanently until toggled back.
        state.disabled = true;
        heartbeat::write_heartbeat_state(&state);
        eprintln!("[heartbeat] toggled off (disabled)");
    }

    Ok(())
}

/// Handle `Command::HeartbeatStatus` — print heartbeat status for tmux status bar.
///
/// Pure display: reads state file, prints kaomoji + countdown. Never fires anything.
pub fn handle_heartbeat_status() -> Result<()> {
    let now = now_secs();

    let state = heartbeat::read_heartbeat_state();

    // No state file yet — show neutral face with placeholder countdown.
    if state.last_beat_ts == 0 && !state.disabled {
        print!("#[fg=colour245](^_^) --#[default]");
        return Ok(());
    }

    // Permanently disabled.
    if state.disabled {
        print!("#[fg=colour240](x_x)#[default]");
        return Ok(());
    }

    let secs_since_beat = now.saturating_sub(state.last_beat_ts);
    let secs_to_next = state.next_beat_ts.saturating_sub(now);

    let face = if secs_since_beat <= 3 {
        // Just fired — excited, bright green.
        format!("#[fg=colour156](^o^) {secs_to_next}s#[default]")
    } else if !state.last_sent {
        // Last scheduled beat was skipped (busy) — sleepy, muted yellow.
        format!("#[fg=colour180](-_-) {secs_to_next}s#[default]")
    } else if state.needs_attention {
        // Workers need attention — alarmed, orange.
        format!("#[fg=colour214](o_O)! {secs_to_next}s#[default]")
    } else {
        // Normal — happy, calm green.
        format!("#[fg=colour114](^_^) {secs_to_next}s#[default]")
    };

    print!("{face}");
    Ok(())
}
