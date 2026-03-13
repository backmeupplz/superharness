use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write as _;
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

/// Append one event to the event log file using JSONL format (one JSON object
/// per line).  This is an O(1) append — no read-all-rewrite required.
///
/// Creates the file if it does not exist.
pub fn log_event(kind: EventKind, pane: Option<&str>, details: &str) -> Result<()> {
    let path = events_path();
    let event = Event {
        timestamp: now_unix(),
        kind,
        pane: pane.map(|s| s.to_string()),
        details: details.to_string(),
    };
    let line = serde_json::to_string(&event).context("failed to serialize event")?;

    // Open file for append (creates it if missing).
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open events file for append: {}", path.display()))?;

    writeln!(file, "{line}")
        .with_context(|| format!("failed to write event line: {}", path.display()))?;

    Ok(())
}

/// Read all events from the event log.
///
/// Supports both legacy JSON-array format and the new JSONL format (one object
/// per line), so existing logs continue to work after an upgrade.
///
/// Returns an empty vec if the file is missing.
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

    // Detect legacy format: if the file starts with '[' it is a JSON array.
    if content.trim_start().starts_with('[') {
        let events: Vec<Event> = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse events file (array): {}", path.display()))?;
        return Ok(events);
    }

    // JSONL format: one JSON object per line, skip blank lines.
    let mut events = Vec::new();
    for (lineno, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: Event = serde_json::from_str(trimmed).with_context(|| {
            format!(
                "failed to parse event on line {} of {}: {}",
                lineno + 1,
                path.display(),
                trimmed
            )
        })?;
        events.push(event);
    }
    Ok(events)
}
