use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::heartbeat;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Handle `Command::Heartbeat`.
pub fn handle_heartbeat(snooze: Option<u64>) -> Result<()> {
    let now = now_secs();

    if let Some(secs) = snooze {
        // Snooze mode: update snooze_until WITHOUT sending a heartbeat.
        // Preserve the disabled flag — snooze is independent of toggle.
        let state = heartbeat::read_heartbeat_state();
        let snooze_until = now + secs;
        heartbeat::write_heartbeat_state_full(
            state.last_beat_ts,
            state.interval_secs,
            state.last_sent,
            state.needs_attention,
            snooze_until,
            state.disabled,
        );
        eprintln!("[heartbeat] snoozed for {secs}s (until unix {snooze_until})");
    } else {
        // Immediate heartbeat: run idle checks and send if %0 is ready.
        // Respects snooze/toggle — does NOT clear it on success.
        match heartbeat::heartbeat() {
            Ok(true) => {
                eprintln!("[heartbeat] sent [HEARTBEAT] to %0");
            }
            Ok(false) => {
                eprintln!("[heartbeat] skipped — %0 is busy or snoozed");
            }
            Err(e) => {
                eprintln!("[heartbeat] error: {e}");
            }
        }
    }

    Ok(())
}

/// Handle `Command::HeartbeatToggle`.
pub fn handle_heartbeat_toggle() -> Result<()> {
    let state = heartbeat::read_heartbeat_state();

    if state.disabled {
        // Currently disabled — re-enable by clearing the disabled flag.
        // Reset timestamps so the countdown starts fresh from now instead of
        // showing '0s' forever (next_beat_ts would still be in the past otherwise).
        let now = now_secs();
        let interval = if state.interval_secs == 0 {
            30
        } else {
            state.interval_secs
        };
        heartbeat::write_heartbeat_state_full(
            now, // last_beat_ts = now → next_beat_ts = now + interval
            interval,
            state.last_sent,
            state.needs_attention,
            state.snooze_until,
            false, // clear disabled
        );
        eprintln!("[heartbeat] toggled on (resumed)");
    } else {
        // Currently enabled — disable by setting the disabled flag.
        heartbeat::write_heartbeat_state_full(
            state.last_beat_ts,
            state.interval_secs,
            state.last_sent,
            state.needs_attention,
            state.snooze_until,
            true, // set disabled
        );
        eprintln!("[heartbeat] toggled off (disabled)");
    }

    Ok(())
}

/// Handle `Command::HeartbeatStatus` — print heartbeat status for tmux status bar.
pub fn handle_heartbeat_status() -> Result<()> {
    let now = now_secs();

    let state = heartbeat::read_heartbeat_state();

    if state.last_beat_ts == 0 && state.snooze_until == 0 && !state.disabled {
        // No heartbeat state file yet.
        print!("♥ --");
        return Ok(());
    }

    // Permanent toggle-off takes priority over timed snooze in display.
    if state.disabled {
        print!("♡ ‖");
        return Ok(());
    }

    // Timed snooze display.
    if state.snooze_until > now {
        let remaining = state.snooze_until - now;
        print!("‖ {remaining}s");
        return Ok(());
    }

    let secs_since_beat = now.saturating_sub(state.last_beat_ts);
    let secs_to_next = state.next_beat_ts.saturating_sub(now);

    let emoji = if secs_since_beat <= 3 {
        // Just fired — big heart (beat effect).
        "❤"
    } else if !state.last_sent {
        // Last beat was skipped (busy) — hollow heart.
        "♡"
    } else if state.needs_attention {
        // Flashing: alternate small/big heart every 5 seconds for a beating effect.
        if (now % 10) < 5 {
            "♥"
        } else {
            "❤"
        }
    } else {
        // Normal — small heart.
        "♥"
    };

    print!("{emoji} {secs_to_next}s");
    Ok(())
}
