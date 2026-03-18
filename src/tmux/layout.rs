use anyhow::Result;

use crate::layout;

use super::{orchestrator_pane_id, tmux, tmux_ok, SESSION};

// ---------------------------------------------------------------------------
// Smart layout helpers
// ---------------------------------------------------------------------------

/// Build a [`layout::PaneLayout`] list from the panes currently visible in
/// the main window (window 0), with no pane flagged as needing attention.
fn main_window_pane_layouts() -> Vec<layout::PaneLayout> {
    let orch_id = orchestrator_pane_id();
    let output = match tmux(&[
        "list-panes",
        "-t",
        &format!("{SESSION}:0"),
        "-F",
        "#{pane_id}",
    ]) {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let id = line.splitn(2, '\t').next().unwrap_or("").to_string();
            let is_orch = id == orch_id;
            layout::PaneLayout {
                is_orchestrator: is_orch,
                needs_attention: false,
                id,
            }
        })
        .collect()
}

/// Apply the smart layout to the current main window without any special
/// attention pane.  Called after `spawn`, `show`/`surface`, and `compact_panes`.
pub fn smart_layout() -> Result<()> {
    let (term_w, term_h) = super::get_terminal_size();
    let panes = main_window_pane_layouts();
    let engine = layout::LayoutEngine::new(term_w, term_h, panes);
    engine.apply()?;
    // Enforce minimum readable pane size after every layout change.
    let _ = layout::enforce_min_pane_size();
    Ok(())
}

/// Apply the smart layout, treating `attention_pane` as needing extra space.
/// If `attention_pane` is currently in a background tab it is surfaced first.
///
/// This is the primary entry point used by `watch.rs` when a pane is detected
/// as `Waiting` (i.e. asking a question or waiting for permission).
pub fn smart_layout_with_attention(attention_pane: Option<&str>) -> Result<()> {
    // 1. Surface the pane if it is in a background window.
    if let Some(pane) = attention_pane {
        let main_panes = main_window_pane_layouts();
        let is_in_main = main_panes.iter().any(|p| p.id == pane);
        if !is_in_main {
            eprintln!("[layout] surfacing attention pane {pane} from background to main window");
            let _ = super::surface(pane);
        }
    }

    // 2. Build pane list with the attention pane flagged.
    let (term_w, term_h) = super::get_terminal_size();
    let panes: Vec<layout::PaneLayout> = main_window_pane_layouts()
        .into_iter()
        .map(|mut p| {
            if attention_pane.map(|ap| ap == p.id).unwrap_or(false) {
                p.needs_attention = true;
            }
            p
        })
        .collect();

    let engine = layout::LayoutEngine::new(term_w, term_h, panes);
    engine.apply()
}

/// Auto-compact the main window: if more than 4 worker panes are visible,
/// move excess panes (highest pane_index, never the orchestrator) to background tabs.
/// Called automatically after each `spawn`.
pub fn auto_compact() -> Result<()> {
    let orch_id = orchestrator_pane_id();

    // List panes in main window (window 0) with their indices and titles
    let output = match tmux(&[
        "list-panes",
        "-t",
        &format!("{SESSION}:0"),
        "-F",
        "#{pane_id}\t#{pane_index}\t#{pane_title}",
    ]) {
        Ok(o) => o,
        Err(_) => return Ok(()), // Session or window not available yet
    };

    let mut panes: Vec<(String, u32, String)> = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let id = parts[0].to_string();
        let index: u32 = parts[1].parse().unwrap_or(0);
        let title = parts[2].to_string();
        panes.push((id, index, title));
    }

    // Exclude orchestrator, sort remaining by pane_index ascending
    let mut workers: Vec<(String, u32, String)> = panes
        .into_iter()
        .filter(|(id, _, _)| *id != orch_id)
        .collect();
    workers.sort_by_key(|(_, idx, _)| *idx);

    // Dynamic threshold based on terminal width
    let (term_w, _) = super::get_terminal_size();
    let max_workers_visible = layout::max_workers_visible(term_w);

    if workers.len() > max_workers_visible {
        let excess = workers.len() - max_workers_visible;
        // Move the highest-index panes (last in sorted order) to background
        let to_move: Vec<_> = workers.into_iter().rev().take(excess).collect();
        for (id, _, title) in to_move {
            let tab_name: String = title.chars().take(20).collect();
            let tab_name = tab_name.trim().to_string();
            let tab_name = if tab_name.is_empty() {
                "worker".to_string()
            } else {
                tab_name
            };
            let _ = tmux_ok(&["break-pane", "-s", &id, "-d", "-n", &tab_name]);
        }
    }

    // Enforce minimum readable pane size after compaction.
    let _ = layout::enforce_min_pane_size();

    Ok(())
}

/// Compact the main window: move any pane (except orchestrator) that is too small
/// (width < term_w/3 or height < term_h/3) to a background tab.
/// Returns (moved_count, remaining_visible_count).
pub fn compact_panes() -> Result<(usize, usize)> {
    let orch_id = orchestrator_pane_id();
    let (term_w, term_h) = super::get_terminal_size();

    // List all panes across all windows with dimensions and window index
    let output = match tmux(&[
        "list-panes",
        "-t",
        SESSION,
        "-a",
        "-F",
        "#{pane_id}\t#{pane_width}\t#{pane_height}\t#{window_index}\t#{pane_title}",
    ]) {
        Ok(o) => o,
        Err(_) => return Ok((0, 0)),
    };

    let mut to_move: Vec<(String, String)> = Vec::new(); // (id, tab_name)
    let mut remaining = 0usize;

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(5, '\t').collect();
        if parts.len() < 5 {
            continue;
        }
        let id = parts[0];
        let width: u32 = parts[1].parse().unwrap_or(0);
        let height: u32 = parts[2].parse().unwrap_or(0);
        let window_index: u32 = parts[3].parse().unwrap_or(999);
        let title = parts[4];

        // Only process panes in the main window (window 0), skip orchestrator
        if window_index != 0 || id == orch_id {
            continue;
        }

        // Use layout-engine aware thresholds: a pane is "too small" when its
        // width is less than what the strategy would allocate per worker, or
        // its height is less than 1/4 of the terminal height.
        let min_w = term_w / (layout::max_workers_visible(term_w) as u32 + 1);
        let min_h = term_h / 4;
        if width < min_w || height < min_h {
            let tab_name: String = title.chars().take(20).collect();
            let tab_name = tab_name.trim().to_string();
            let tab_name = if tab_name.is_empty() {
                "worker".to_string()
            } else {
                tab_name
            };
            to_move.push((id.to_string(), tab_name));
        } else {
            remaining += 1;
        }
    }

    let mut moved = 0usize;
    for (id, tab_name) in &to_move {
        if tmux_ok(&["break-pane", "-s", id, "-d", "-n", tab_name]).is_ok() {
            moved += 1;
        }
    }

    // Re-apply smart layout to main window if anything was moved
    if moved > 0 {
        let _ = smart_layout();
    }

    // Enforce minimum readable pane size regardless of whether compaction moved anything.
    let _ = layout::enforce_min_pane_size();

    Ok((moved, remaining))
}

/// Resize a pane.
pub fn resize(pane: &str, direction: &str, amount: u32) -> Result<()> {
    let flag = match direction.to_uppercase().as_str() {
        "U" => "-U",
        "D" => "-D",
        "L" => "-L",
        "R" => "-R",
        other => anyhow::bail!("invalid direction: {other} (use U, D, L, R)"),
    };
    tmux_ok(&["resize-pane", "-t", pane, flag, &amount.to_string()])
}

/// Apply a layout preset.
pub fn layout(name: &str) -> Result<()> {
    tmux_ok(&["select-layout", "-t", SESSION, name])
}
