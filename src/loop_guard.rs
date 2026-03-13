use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::util::{hash_string, now_unix};

const DEFAULT_WINDOW_SIZE: usize = 10;
const REPEAT_THRESHOLD: u32 = 3;

/// A single recorded action for a pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub pane_id: String,
    pub action_type: String,
    pub content_hash: u64,
    pub timestamp: u64,
}

/// Result of loop detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetection {
    pub detected: bool,
    pub pane_id: String,
    pub repeated_action: String,
    pub count: u32,
}

/// Per-pane action history and window configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoopState {
    pub actions: Vec<ActionRecord>,
    pub window_size: usize,
}

/// Full persisted state: a map from pane_id to LoopState.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistState {
    panes: std::collections::HashMap<String, LoopState>,
}

fn data_path() -> PathBuf {
    let base = dirs_path();
    base.join("loop_state.json")
}

fn dirs_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
}

fn load_state() -> Result<PersistState> {
    let path = data_path();
    if !path.exists() {
        return Ok(PersistState::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read loop state from {}", path.display()))?;
    let state: PersistState =
        serde_json::from_str(&content).with_context(|| "failed to parse loop state JSON")?;
    Ok(state)
}

fn save_state(state: &PersistState) -> Result<()> {
    let dir = dirs_path();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;
    let path = data_path();
    let content = serde_json::to_string_pretty(state).context("failed to serialize loop state")?;
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write loop state to {}", path.display()))?;
    Ok(())
}

/// Analyze actions in the sliding window for repetition or oscillation.
fn analyze_window(
    pane_id: &str,
    actions: &[ActionRecord],
    window_size: usize,
) -> Option<LoopDetection> {
    if actions.is_empty() {
        return None;
    }

    // Take the last `window_size` actions
    let start = actions.len().saturating_sub(window_size);
    let window = &actions[start..];

    // Check for repeated (action_type, content_hash) triples
    let mut counts: std::collections::HashMap<(String, u64), u32> =
        std::collections::HashMap::new();
    for record in window {
        let key = (record.action_type.clone(), record.content_hash);
        *counts.entry(key).or_insert(0) += 1;
    }

    for ((action_type, _hash), count) in &counts {
        if *count >= REPEAT_THRESHOLD {
            return Some(LoopDetection {
                detected: true,
                pane_id: pane_id.to_string(),
                repeated_action: action_type.clone(),
                count: *count,
            });
        }
    }

    // Check for oscillation: A->B->A->B pattern (at least 2 full cycles = 4 elements)
    if window.len() >= 4 {
        let last = window.len();
        // Look for A->B->A->B by checking pairs at even distance
        let a0 = &window[last - 4];
        let b0 = &window[last - 3];
        let a1 = &window[last - 2];
        let b1 = &window[last - 1];

        let a_same = a0.action_type == a1.action_type && a0.content_hash == a1.content_hash;
        let b_same = b0.action_type == b1.action_type && b0.content_hash == b1.content_hash;
        let ab_different = a0.action_type != b0.action_type || a0.content_hash != b0.content_hash;

        if a_same && b_same && ab_different {
            return Some(LoopDetection {
                detected: true,
                pane_id: pane_id.to_string(),
                repeated_action: format!("{} <-> {}", a0.action_type, b0.action_type),
                count: 2,
            });
        }
    }

    None
}

/// Record an action for a pane and check for loop patterns.
/// Returns Some(LoopDetection) if a loop is detected.
pub fn record_action(
    pane_id: &str,
    action_type: &str,
    content: &str,
) -> Result<Option<LoopDetection>> {
    let mut state = load_state()?;

    let pane_state = state
        .panes
        .entry(pane_id.to_string())
        .or_insert_with(|| LoopState {
            actions: Vec::new(),
            window_size: DEFAULT_WINDOW_SIZE,
        });

    let record = ActionRecord {
        pane_id: pane_id.to_string(),
        action_type: action_type.to_string(),
        content_hash: hash_string(content),
        timestamp: now_unix(),
    };

    pane_state.actions.push(record);

    // Trim history to keep memory bounded (keep 2x window to allow analysis)
    let max_history = pane_state.window_size * 2;
    if pane_state.actions.len() > max_history {
        let drain_count = pane_state.actions.len() - max_history;
        pane_state.actions.drain(..drain_count);
    }

    let window_size = pane_state.window_size;
    let detection = analyze_window(pane_id, &pane_state.actions, window_size);

    save_state(&state)?;

    Ok(detection)
}

/// Clear loop history for a specific pane.
pub fn clear_pane(pane_id: &str) -> Result<()> {
    let mut state = load_state()?;
    state.panes.remove(pane_id);
    save_state(&state)?;
    Ok(())
}

/// Get current loop detection status for a pane without recording a new action.
pub fn get_loop_status(pane_id: &str) -> Result<Option<LoopDetection>> {
    let state = load_state()?;

    let pane_state = match state.panes.get(pane_id) {
        Some(s) => s,
        None => return Ok(None),
    };

    Ok(analyze_window(
        pane_id,
        &pane_state.actions,
        pane_state.window_size,
    ))
}

/// Get loop detection status for all known panes.
#[allow(dead_code)]
pub fn get_all_loop_status() -> Result<Vec<LoopDetection>> {
    let state = load_state()?;
    let mut results = Vec::new();

    for (pane_id, pane_state) in &state.panes {
        if let Some(detection) =
            analyze_window(pane_id, &pane_state.actions, pane_state.window_size)
        {
            results.push(detection);
        }
    }

    Ok(results)
}

/// Get all known pane IDs with their action counts (even if no loop detected).
pub fn get_all_panes() -> Result<Vec<(String, usize)>> {
    let state = load_state()?;
    Ok(state
        .panes
        .iter()
        .map(|(id, s)| (id.clone(), s.actions.len()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(pane: &str, action: &str, content: &str, ts: u64) -> ActionRecord {
        ActionRecord {
            pane_id: pane.to_string(),
            action_type: action.to_string(),
            content_hash: hash_string(content),
            timestamp: ts,
        }
    }

    #[test]
    fn test_no_loop_small_history() {
        let actions = vec![
            make_record("%1", "send", "hello", 1),
            make_record("%1", "send", "world", 2),
        ];
        assert!(analyze_window("%1", &actions, 10).is_none());
    }

    #[test]
    fn test_repeat_detection() {
        let actions = vec![
            make_record("%1", "send", "retry", 1),
            make_record("%1", "send", "retry", 2),
            make_record("%1", "send", "retry", 3),
        ];
        let result = analyze_window("%1", &actions, 10);
        assert!(result.is_some());
        let det = result.unwrap();
        assert!(det.detected);
        assert_eq!(det.count, 3);
    }

    #[test]
    fn test_oscillation_detection() {
        let actions = vec![
            make_record("%1", "send", "yes", 1),
            make_record("%1", "send", "no", 2),
            make_record("%1", "send", "yes", 3),
            make_record("%1", "send", "no", 4),
        ];
        let result = analyze_window("%1", &actions, 10);
        assert!(result.is_some());
        let det = result.unwrap();
        assert!(det.detected);
    }

    #[test]
    fn test_no_oscillation_different_content() {
        let actions = vec![
            make_record("%1", "send", "a", 1),
            make_record("%1", "send", "b", 2),
            make_record("%1", "send", "c", 3),
            make_record("%1", "send", "d", 4),
        ];
        assert!(analyze_window("%1", &actions, 10).is_none());
    }
}
