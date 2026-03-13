use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::project;
use crate::util::{generate_id, now_unix};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTask {
    pub id: String,
    pub task: String,
    pub dir: String,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub name: Option<String>,
    /// Override the AI harness for this worker (opencode / claude / codex).
    /// When absent the configured default is used.
    #[serde(default)]
    pub harness: Option<String>,
    /// Pane IDs that must finish (i.e. no longer appear in tmux list) before this task can run
    pub depends_on: Vec<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Store {
    tasks: Vec<PendingTask>,
}

fn store_path() -> Result<PathBuf> {
    Ok(project::get_project_state_dir()?.join("pending_tasks.json"))
}

fn load() -> Result<Store> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(Store::default());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let store: Store = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(store)
}

fn save(store: &Store) -> Result<()> {
    let path = store_path()?;
    // The directory is created by get_project_state_dir() called inside store_path()
    let content = serde_json::to_string_pretty(store).context("failed to serialize store")?;
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Input parameters for [`add_task`].
///
/// Using a struct avoids the `clippy::too_many_arguments` lint and makes
/// call sites more readable when only some fields are populated.
pub struct PendingTaskInput {
    pub task: String,
    pub dir: String,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub name: Option<String>,
    pub harness: Option<String>,
    pub depends_on: Vec<String>,
}

/// Add a new pending task. Returns the generated task ID.
pub fn add_task(input: PendingTaskInput) -> Result<String> {
    let mut store = load()?;
    let id = generate_id("task");
    store.tasks.push(PendingTask {
        id: id.clone(),
        task: input.task,
        dir: input.dir,
        model: input.model,
        mode: input.mode,
        name: input.name,
        harness: input.harness,
        depends_on: input.depends_on,
        created_at: now_unix(),
    });
    save(&store)?;
    Ok(id)
}

/// Return all pending tasks.
pub fn list_tasks() -> Result<Vec<PendingTask>> {
    Ok(load()?.tasks)
}

/// Remove a task by ID.
pub fn remove_task(id: &str) -> Result<()> {
    let mut store = load()?;
    store.tasks.retain(|t| t.id != id);
    save(&store)
}

/// Given the set of currently-active pane IDs, return tasks whose every dependency is gone.
pub fn ready_tasks(active_pane_ids: &[String]) -> Result<Vec<PendingTask>> {
    let store = load()?;
    let ready = store
        .tasks
        .into_iter()
        .filter(|t| {
            t.depends_on
                .iter()
                .all(|dep| !active_pane_ids.contains(dep))
        })
        .collect();
    Ok(ready)
}
