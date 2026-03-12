use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn memory_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("cannot determine home directory")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
        .join("memory"))
}

fn memory_file(pane_id: &str) -> Result<PathBuf> {
    let safe_id = pane_id.trim_start_matches('%');
    Ok(memory_dir()?.join(format!("pane-{safe_id}.json")))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PaneMemory {
    pub pane_id: String,
    pub entries: HashMap<String, String>,
}

fn load(pane_id: &str) -> Result<PaneMemory> {
    let path = memory_file(pane_id)?;
    if !path.exists() {
        return Ok(PaneMemory {
            pane_id: pane_id.to_string(),
            entries: HashMap::new(),
        });
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read memory file: {}", path.display()))?;
    let mem: PaneMemory = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse memory file: {}", path.display()))?;
    Ok(mem)
}

fn save_mem(mem: &PaneMemory) -> Result<()> {
    let path = memory_file(&mem.pane_id)?;
    let dir = path.parent().expect("memory file must have a parent dir");
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create memory directory: {}", dir.display()))?;
    let content = serde_json::to_string_pretty(mem).context("failed to serialize memory")?;
    fs::write(&path, content)
        .with_context(|| format!("failed to write memory file: {}", path.display()))?;
    Ok(())
}

/// Store a key-value fact for a pane.
pub fn set(pane_id: &str, key: &str, value: &str) -> Result<()> {
    let mut mem = load(pane_id)?;
    mem.pane_id = pane_id.to_string();
    mem.entries.insert(key.to_string(), value.to_string());
    save_mem(&mem)
}

/// Retrieve all key-value facts for a pane.
pub fn get_all(pane_id: &str) -> Result<PaneMemory> {
    load(pane_id)
}
