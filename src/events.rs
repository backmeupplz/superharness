use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    WorkerSpawned,
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
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
        .join("events.json")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append one event to the event log file (creates it if it doesn't exist).
pub fn log_event(kind: EventKind, pane: Option<&str>, details: &str) -> Result<()> {
    let path = events_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create events directory: {}", parent.display()))?;
    }

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

/// Return only events with timestamp >= since.
pub fn events_since(since: u64) -> Result<Vec<Event>> {
    let all = load_events()?;
    Ok(all.into_iter().filter(|e| e.timestamp >= since).collect())
}
