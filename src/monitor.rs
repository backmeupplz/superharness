use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// State persisted between monitor runs.
#[derive(Serialize, Deserialize, Default)]
pub struct MonitorState {
    /// Number of consecutive checks where output was unchanged per pane.
    pub stall_counts: HashMap<String, u32>,
    /// Hash of the last seen output per pane.
    pub last_output_hash: HashMap<String, u64>,
    /// Number of recovery attempts already made per pane.
    pub recovery_attempts: HashMap<String, u32>,
}

fn state_path() -> PathBuf {
    let base = dirs_home().unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(".local")
        .join("share")
        .join("superharness")
        .join("monitor_state.json")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

pub fn load_state() -> MonitorState {
    let path = state_path();
    if !path.exists() {
        return MonitorState::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => MonitorState::default(),
    }
}
