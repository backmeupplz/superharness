use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::project;

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

fn state_path() -> anyhow::Result<PathBuf> {
    Ok(project::get_project_state_dir()?.join("monitor_state.json"))
}

pub fn load_state() -> MonitorState {
    let path = match state_path() {
        Ok(p) => p,
        Err(_) => return MonitorState::default(),
    };
    if !path.exists() {
        return MonitorState::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => MonitorState::default(),
    }
}
