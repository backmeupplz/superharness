use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::project;
use crate::util::now_unix;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    WorkerSpawned,
    WorkerCompleted,
    WorkerKilled,
    WorkerStalled,
    WorkerRecovered,
    DecisionQueued,
    DecisionCleared,
    ModeChanged,
    Pulse,
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EventKind::WorkerSpawned => "WorkerSpawned",
            EventKind::WorkerCompleted => "WorkerCompleted",
            EventKind::WorkerKilled => "WorkerKilled",
            EventKind::WorkerStalled => "WorkerStalled",
            EventKind::WorkerRecovered => "WorkerRecovered",
            EventKind::DecisionQueued => "DecisionQueued",
            EventKind::DecisionCleared => "DecisionCleared",
            EventKind::ModeChanged => "ModeChanged",
            EventKind::Pulse => "Pulse",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: u64,
    pub kind: EventKind,
    pub pane: Option<String>,
    pub details: String,
}

fn events_path() -> PathBuf {
    // Use project-local state dir; fall back to /tmp on error
    project::get_project_state_dir()
        .map(|d| d.join("events.json"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/superharness-events.json"))
}

/// Append one event to the event log file (creates it if it doesn't exist).
pub fn log_event(kind: EventKind, pane: Option<&str>, details: &str) -> Result<()> {
    let path = events_path();
    // Directory is already created by get_project_state_dir() inside events_path()
    let mut events = load_events().unwrap_or_default();
    events.push(Event {
        timestamp: now_unix(),
        kind,
        pane: pane.map(|s| s.to_string()),
        details: details.to_string(),
    });

    let json = serde_json::to_string_pretty(&events).context("failed to serialize events")?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write events file: {}", path.display()))?;
    Ok(())
}

/// Read all events from the event log (returns empty vec if file is missing).
pub fn load_events() -> Result<Vec<Event>> {
    let path = events_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read events file: {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let events: Vec<Event> = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse events file: {}", path.display()))?;
    Ok(events)
}
