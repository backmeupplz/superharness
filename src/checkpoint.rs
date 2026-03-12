use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn checkpoints_base_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("cannot determine home directory")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
        .join("checkpoints"))
}

fn pane_checkpoint_dir(pane_id: &str) -> Result<PathBuf> {
    // Strip leading '%' for filesystem safety, but keep it recognisable
    let safe_id = pane_id.trim_start_matches('%');
    Ok(checkpoints_base_dir()?.join(format!("pane-{safe_id}")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique ID: "<pane_id>/<timestamp>"
    pub id: String,
    pub pane_id: String,
    pub timestamp: u64,
    pub task_title: String,
    pub note: Option<String>,
    /// Last N lines of pane output at the time of checkpoint
    pub last_output: String,
}

/// Save a checkpoint for `pane_id`.
/// `task_title` – typically the pane title (first 50 chars of task).
/// `pane_output` – full captured output; we store the last 100 lines.
/// `note` – optional human note.
pub fn save(
    pane_id: &str,
    task_title: &str,
    pane_output: &str,
    note: Option<&str>,
) -> Result<Checkpoint> {
    let ts = now_unix();
    let dir = pane_checkpoint_dir(pane_id)?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create checkpoint dir: {}", dir.display()))?;

    // Keep only the last 100 lines to keep files small
    let last_output: String = pane_output
        .lines()
        .rev()
        .take(100)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    let checkpoint = Checkpoint {
        id: format!("{}/{}", pane_id, ts),
        pane_id: pane_id.to_string(),
        timestamp: ts,
        task_title: task_title.to_string(),
        note: note.map(|s| s.to_string()),
        last_output,
    };

    let path = dir.join(format!("{ts}.json"));
    let content =
        serde_json::to_string_pretty(&checkpoint).context("failed to serialize checkpoint")?;
    fs::write(&path, content)
        .with_context(|| format!("failed to write checkpoint: {}", path.display()))?;

    Ok(checkpoint)
}

/// List all checkpoints, optionally filtered to a single pane.
pub fn list(pane_id: Option<&str>) -> Result<Vec<Checkpoint>> {
    let base = checkpoints_base_dir()?;
    if !base.exists() {
        return Ok(vec![]);
    }

    let mut checkpoints = Vec::new();

    // Enumerate pane subdirectories
    for entry in
        fs::read_dir(&base).with_context(|| format!("failed to read {}", base.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // If filtering by pane, skip non-matching dirs
        if let Some(pid) = pane_id {
            let safe_id = pid.trim_start_matches('%');
            let expected_dir = format!("pane-{safe_id}");
            if path.file_name().and_then(|n| n.to_str()) != Some(&expected_dir) {
                continue;
            }
        }

        // Read each .json file in the pane dir
        for file_entry in
            fs::read_dir(&path).with_context(|| format!("failed to read {}", path.display()))?
        {
            let file_entry = file_entry?;
            let file_path = file_entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(&file_path).with_context(|| {
                format!("failed to read checkpoint file: {}", file_path.display())
            })?;
            match serde_json::from_str::<Checkpoint>(&content) {
                Ok(cp) => checkpoints.push(cp),
                Err(e) => eprintln!(
                    "warning: skipping malformed checkpoint {}: {e}",
                    file_path.display()
                ),
            }
        }
    }

    // Sort by timestamp ascending
    checkpoints.sort_by_key(|c| c.timestamp);
    Ok(checkpoints)
}

/// Load a single checkpoint by its composite ID ("<pane_id>/<timestamp>").
pub fn load_by_id(id: &str) -> Result<Checkpoint> {
    // id format: "<pane_id>/<timestamp>"  e.g. "%5/1741234567"
    let parts: Vec<&str> = id.splitn(2, '/').collect();
    if parts.len() != 2 {
        anyhow::bail!("invalid checkpoint id format; expected '<pane_id>/<timestamp>'");
    }
    let pane_id = parts[0];
    let ts_str = parts[1];
    let ts: u64 = ts_str
        .parse()
        .with_context(|| format!("invalid timestamp in checkpoint id: {ts_str}"))?;

    let dir = pane_checkpoint_dir(pane_id)?;
    let path = dir.join(format!("{ts}.json"));

    if !path.exists() {
        anyhow::bail!("checkpoint not found: {}", path.display());
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read checkpoint: {}", path.display()))?;
    let cp: Checkpoint = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse checkpoint: {}", path.display()))?;
    Ok(cp)
}
