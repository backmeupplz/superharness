//! Smart layout engine for SuperHarness tmux pane management.
//!
//! Chooses a layout strategy based on terminal size and pane count,
//! giving extra space to panes that need attention (e.g. waiting for approval).
//!
//! # Strategy selection
//! | Panes visible | Wide (≥120) | Narrow (<120) |
//! |---------------|-------------|---------------|
//! | 1             | Single      | Single        |
//! | 2             | SideBySide  | SideBySide-V  |
//! | 3             | MainWithStack | MainWithStack-H |
//! | 4             | Grid2x2     | Grid2x2       |
//! | 5+            | OrchestratorMain | OrchestratorMain-H |
//!
//! # Attention behaviour
//! When a pane has `needs_attention = true`, the engine:
//! 1. Surfaces it to the main window if it was in a background tab.
//! 2. Gives it extra column/row space (shrinks orchestrator slightly).

use std::process::Command;

use crate::tmux::SESSION;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Per-pane metadata used by the layout engine to make sizing decisions.
#[derive(Debug, Clone)]
pub struct PaneLayout {
    /// Tmux pane ID (e.g. `"%3"`).
    pub id: String,
    /// True when the pane is waiting for human input / has a question.
    /// Attention panes are given more screen space and surfaced to the front.
    pub needs_attention: bool,
    /// True when this is the orchestrator pane (`%0`).
    pub is_orchestrator: bool,
}

/// Layout strategies the engine can apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutStrategy {
    /// 1 pane — full screen.
    Single,
    /// 2 panes — orchestrator left 40 %, worker right 60 % (wide)
    ///           or orchestrator top 40 %, worker bottom 60 % (narrow).
    SideBySide,
    /// 3 panes — orchestrator left 35 % with two workers stacked on the right
    ///           (wide), or orchestrator top 35 % with two workers side-by-side
    ///           below (narrow).
    MainWithStack,
    /// 4 panes — 2 × 2 grid (tmux `tiled`).
    Grid2x2,
    /// 5 + panes — orchestrator left 35 % (wide) or top 40 % (narrow),
    ///             workers fill the remaining space.
    OrchestratorMain,
}

/// The layout engine.  Constructed from terminal dimensions and the current
/// list of visible panes, then `apply()` is called to reconfigure tmux.
pub struct LayoutEngine {
    pub term_width: u32,
    pub term_height: u32,
    pub visible_panes: Vec<PaneLayout>,
}

impl LayoutEngine {
    pub fn new(term_width: u32, term_height: u32, visible_panes: Vec<PaneLayout>) -> Self {
        Self {
            term_width,
            term_height,
            visible_panes,
        }
    }

    /// Choose a strategy based on current state.
    pub fn choose_strategy(&self) -> LayoutStrategy {
        choose_strategy(self.visible_panes.len(), self.term_width, self.term_height)
    }

    /// Apply the chosen strategy to the main tmux window.
    pub fn apply(&self) -> anyhow::Result<()> {
        let strategy = self.choose_strategy();
        apply_strategy(
            strategy,
            &self.visible_panes,
            self.term_width,
            self.term_height,
        )
    }
}

// ---------------------------------------------------------------------------
// Strategy selection
// ---------------------------------------------------------------------------

/// Choose a [`LayoutStrategy`] based on pane count and terminal width.
pub fn choose_strategy(pane_count: usize, _term_width: u32, _term_height: u32) -> LayoutStrategy {
    match pane_count {
        0 | 1 => LayoutStrategy::Single,
        2 => LayoutStrategy::SideBySide,
        3 => LayoutStrategy::MainWithStack,
        4 => LayoutStrategy::Grid2x2,
        _ => LayoutStrategy::OrchestratorMain,
    }
}

/// Dynamic maximum number of worker panes to keep visible in the main window
/// based on terminal width.
pub fn max_workers_visible(term_width: u32) -> usize {
    match term_width {
        w if w >= 200 => 5,
        w if w >= 120 => 4,
        w if w >= 80 => 3,
        _ => 2,
    }
}

// ---------------------------------------------------------------------------
// Strategy application
// ---------------------------------------------------------------------------

/// Apply a [`LayoutStrategy`] to the main tmux window, then expand any
/// panes that have `needs_attention = true`.
pub fn apply_strategy(
    strategy: LayoutStrategy,
    panes: &[PaneLayout],
    term_width: u32,
    term_height: u32,
) -> anyhow::Result<()> {
    let session_win = format!("{SESSION}:0");
    let is_wide = term_width >= 120;

    // ------------------------------------------------------------------
    // 1.  Select %0 as the active pane so tmux main-* layouts treat it
    //     as the "main" pane (tmux uses the active pane for that role).
    // ------------------------------------------------------------------
    run_tmux(&["select-pane", "-t", "%0"]);

    // ------------------------------------------------------------------
    // 2.  Apply the base tmux layout.
    // ------------------------------------------------------------------
    match strategy {
        LayoutStrategy::Single => {
            run_tmux(&["select-layout", "-t", &session_win, "even-horizontal"]);
        }

        LayoutStrategy::SideBySide => {
            if is_wide {
                // Horizontal split: orchestrator left, worker right.
                run_tmux(&["select-layout", "-t", &session_win, "even-horizontal"]);
                let orch_w = ((term_width as f64 * 0.40) as u32).max(20);
                run_tmux(&["resize-pane", "-t", "%0", "-x", &orch_w.to_string()]);
            } else {
                // Narrow: stack vertically.
                run_tmux(&["select-layout", "-t", &session_win, "even-vertical"]);
                let orch_h = ((term_height as f64 * 0.40) as u32).max(8);
                run_tmux(&["resize-pane", "-t", "%0", "-y", &orch_h.to_string()]);
            }
        }

        LayoutStrategy::MainWithStack => {
            if is_wide {
                // Orchestrator left 35 %, workers stacked vertically on the right.
                run_tmux(&["select-layout", "-t", &session_win, "main-vertical"]);
                let orch_w = ((term_width as f64 * 0.35) as u32).max(20);
                run_tmux(&["resize-pane", "-t", "%0", "-x", &orch_w.to_string()]);
            } else {
                // Narrow: orchestrator top, workers side-by-side below.
                run_tmux(&["select-layout", "-t", &session_win, "main-horizontal"]);
                let orch_h = ((term_height as f64 * 0.35) as u32).max(8);
                run_tmux(&["resize-pane", "-t", "%0", "-y", &orch_h.to_string()]);
            }
        }

        LayoutStrategy::Grid2x2 => {
            run_tmux(&["select-layout", "-t", &session_win, "tiled"]);
        }

        LayoutStrategy::OrchestratorMain => {
            if is_wide {
                // Orchestrator left 35 %, workers fill the right column.
                run_tmux(&["select-layout", "-t", &session_win, "main-vertical"]);
                let orch_w = ((term_width as f64 * 0.35) as u32).max(20);
                run_tmux(&["resize-pane", "-t", "%0", "-x", &orch_w.to_string()]);
            } else {
                // Narrow: orchestrator top 40 %, workers fill the bottom strip.
                run_tmux(&["select-layout", "-t", &session_win, "main-horizontal"]);
                let orch_h = ((term_height as f64 * 0.40) as u32).max(8);
                run_tmux(&["resize-pane", "-t", "%0", "-y", &orch_h.to_string()]);
            }
        }
    }

    // ------------------------------------------------------------------
    // 3.  Expand attention panes (most important behaviour).
    //     We shrink the orchestrator slightly and grow the attention pane
    //     so it stands out and is easy to read/respond to.
    // ------------------------------------------------------------------
    for pane in panes {
        if pane.needs_attention && !pane.is_orchestrator {
            eprintln!(
                "[layout] attention pane {} — expanding (strategy={:?}, wide={is_wide})",
                pane.id, strategy
            );

            match strategy {
                LayoutStrategy::SideBySide if is_wide => {
                    // Shrink orchestrator further so the worker gets more space.
                    let orch_w = ((term_width as f64 * 0.28) as u32).max(20);
                    run_tmux(&["resize-pane", "-t", "%0", "-x", &orch_w.to_string()]);
                }
                LayoutStrategy::SideBySide => {
                    // Narrow: shrink orchestrator top slice.
                    let orch_h = ((term_height as f64 * 0.28) as u32).max(6);
                    run_tmux(&["resize-pane", "-t", "%0", "-y", &orch_h.to_string()]);
                }
                LayoutStrategy::MainWithStack | LayoutStrategy::OrchestratorMain => {
                    // Give the attention pane extra columns on the right.
                    if is_wide {
                        let extra = (term_width / 8).max(5);
                        run_tmux(&["resize-pane", "-t", &pane.id, "-R", &extra.to_string()]);
                    } else {
                        let extra = (term_height / 8).max(3);
                        run_tmux(&["resize-pane", "-t", &pane.id, "-D", &extra.to_string()]);
                    }
                }
                // Grid2x2 / Single: expand by column on the right.
                _ => {
                    let extra = (term_width / 8).max(5);
                    run_tmux(&["resize-pane", "-t", &pane.id, "-R", &extra.to_string()]);
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Minimum pane size enforcement
// ---------------------------------------------------------------------------

/// Minimum readable pane width (columns).
pub const MIN_PANE_COLS: u32 = 60;
/// Minimum readable pane height (rows).
pub const MIN_PANE_ROWS: u32 = 12;

/// Scan all panes in the main window (window 0) and hide any that are smaller
/// than [`MIN_PANE_COLS`] × [`MIN_PANE_ROWS`], except for `%0` (orchestrator).
///
/// Called automatically at the end of `smart_layout()` and `auto_compact()` in
/// `tmux.rs` to ensure the main window never shows unreadably-small panes.
pub fn enforce_min_pane_size() -> anyhow::Result<()> {
    // List all panes across all windows with dimensions and window index.
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            SESSION,
            "-a",
            "-F",
            "#{pane_id} #{pane_width} #{pane_height} #{window_index} #{pane_title}",
        ])
        .output();

    let raw = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Ok(()),
    };

    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        // Split on spaces — title is the rest (may contain spaces) but we only
        // need up to index 4 for title so use splitn(5, ' ').
        let parts: Vec<&str> = line.splitn(5, ' ').collect();
        if parts.len() < 4 {
            continue;
        }

        let id = parts[0];
        let width: u32 = parts[1].parse().unwrap_or(0);
        let height: u32 = parts[2].parse().unwrap_or(0);
        let window_index: u32 = parts[3].parse().unwrap_or(999);
        let title = parts.get(4).copied().unwrap_or("worker");

        // Only enforce in the main window; never touch %0 or background panes.
        if id == "%0" || window_index != 0 {
            continue;
        }

        if width < MIN_PANE_COLS || height < MIN_PANE_ROWS {
            eprintln!(
                "[layout] enforce_min_pane_size: hiding pane {id} ({width}×{height} \
                 below minimum {MIN_PANE_COLS}×{MIN_PANE_ROWS})"
            );
            // Derive a sensible tab name from the pane title.
            let tab_name: String = title.chars().take(20).collect();
            let tab_name = tab_name.trim();
            let tab_name = if tab_name.is_empty() { id } else { tab_name };
            run_tmux(&["break-pane", "-s", id, "-d", "-n", tab_name]);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal tmux helper
// ---------------------------------------------------------------------------

/// Run a tmux command, ignoring errors (layout adjustments are best-effort).
fn run_tmux(args: &[&str]) {
    let _ = Command::new("tmux").args(args).status();
}
