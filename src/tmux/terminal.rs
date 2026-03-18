use serde::Serialize;
use std::process::Command;

use super::{orchestrator_pane_id, SESSION};

/// Information returned by the `terminal-size` subcommand.
#[derive(Serialize)]
pub struct TerminalSizeInfo {
    pub width: u32,
    pub height: u32,
    /// Fixed row count for the orchestrator pane (%0).
    pub main_pane_rows: u32,
    /// Number of non-%0 panes currently visible in window 0.
    pub workers_visible: usize,
    /// Recommended maximum number of workers to show simultaneously,
    /// based on terminal width:
    ///   < 120  → 1
    ///   120–200 → 2
    ///   200–300 → 3
    ///   300+   → 4
    pub recommended_max_workers: usize,
}

/// Compute terminal size metadata and return it as a [`TerminalSizeInfo`].
pub fn terminal_size_info() -> TerminalSizeInfo {
    let (width, height) = get_terminal_size();

    // Count non-orchestrator panes in window 0.
    let orch_id = orchestrator_pane_id();
    let workers_visible: usize = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            &format!("{SESSION}:0"),
            "-F",
            "#{pane_id}",
        ])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty() && l.trim() != orch_id)
                .count()
        })
        .unwrap_or(0);

    let recommended_max_workers = match width {
        w if w >= 300 => 4,
        w if w >= 200 => 3,
        w if w >= 120 => 2,
        _ => 1,
    };

    TerminalSizeInfo {
        width,
        height,
        main_pane_rows: 82,
        workers_visible,
        recommended_max_workers,
    }
}

/// Query the current tmux window dimensions dynamically.
/// Returns (width, height). Falls back to (80, 24) on any error.
pub fn get_terminal_size() -> (u32, u32) {
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-t",
            SESSION,
            "-p",
            "#{window_width} #{window_height}",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            let s = s.trim();
            let mut parts = s.split_whitespace();
            let w: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(80);
            let h: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(24);
            (w, h)
        }
        _ => (80, 24),
    }
}
