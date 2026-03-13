use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Enums ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Failed,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::Failed => write!(f, "failed"),
        }
    }
}

// ── Structs ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub assigned_pane: Option<String>,
    pub worktree_path: Option<String>,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tasks: Vec<PlanTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPlan {
    pub description: String,
    pub repo_path: String,
    pub model: String,
    pub max_concurrent_workers: usize,
    pub current_stage_index: usize,
    pub stages: Vec<Stage>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("cannot determine home directory (HOME not set)")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness"))
}

pub fn plan_file_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("project_plan.json"))
}

#[allow(dead_code)]
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── PlanManager ───────────────────────────────────────────────────────────────

pub struct PlanManager {
    pub path: PathBuf,
}

#[allow(dead_code)]
impl PlanManager {
    pub fn new() -> Result<Self> {
        let dir = data_dir()?;
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create data directory: {}", dir.display()))?;
        let path = dir.join("project_plan.json");
        Ok(Self { path })
    }

    pub fn load(&self) -> Result<Option<ProjectPlan>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read plan file: {}", self.path.display()))?;
        let plan: ProjectPlan = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse plan file: {}", self.path.display()))?;
        Ok(Some(plan))
    }

    pub fn save(&self, plan: &ProjectPlan) -> Result<()> {
        let content = serde_json::to_string_pretty(plan).context("failed to serialize plan")?;
        fs::write(&self.path, &content)
            .with_context(|| format!("failed to write plan file: {}", self.path.display()))?;
        Ok(())
    }

    pub fn current_stage<'a>(plan: &'a ProjectPlan) -> Option<&'a Stage> {
        plan.stages.get(plan.current_stage_index)
    }

    /// Returns (stage_idx, task_idx) of the next pending task in the current stage.
    pub fn next_pending_task(plan: &ProjectPlan) -> Option<(usize, usize)> {
        let stage_idx = plan.current_stage_index;
        let stage = plan.stages.get(stage_idx)?;
        for (task_idx, task) in stage.tasks.iter().enumerate() {
            if task.status == TaskStatus::Pending {
                return Some((stage_idx, task_idx));
            }
        }
        None
    }

    pub fn mark_task_in_progress(
        plan: &mut ProjectPlan,
        stage_idx: usize,
        task_idx: usize,
        pane: &str,
        worktree: &str,
    ) {
        if let Some(stage) = plan.stages.get_mut(stage_idx) {
            if let Some(task) = stage.tasks.get_mut(task_idx) {
                task.status = TaskStatus::InProgress;
                task.assigned_pane = Some(pane.to_string());
                task.worktree_path = Some(worktree.to_string());
                task.started_at = Some(now_unix());
            }
        }
    }

    pub fn mark_task_done(plan: &mut ProjectPlan, stage_idx: usize, task_idx: usize) {
        if let Some(stage) = plan.stages.get_mut(stage_idx) {
            if let Some(task) = stage.tasks.get_mut(task_idx) {
                task.status = TaskStatus::Done;
                task.completed_at = Some(now_unix());
            }
        }
    }

    pub fn mark_task_failed(plan: &mut ProjectPlan, stage_idx: usize, task_idx: usize) {
        if let Some(stage) = plan.stages.get_mut(stage_idx) {
            if let Some(task) = stage.tasks.get_mut(task_idx) {
                task.status = TaskStatus::Failed;
                task.completed_at = Some(now_unix());
            }
        }
    }

    /// If all tasks in the current stage are done or failed, increment current_stage_index.
    /// Returns true if the stage was advanced.
    pub fn advance_stage_if_complete(plan: &mut ProjectPlan) -> bool {
        let stage_idx = plan.current_stage_index;
        if let Some(stage) = plan.stages.get(stage_idx) {
            let all_terminal = stage
                .tasks
                .iter()
                .all(|t| t.status == TaskStatus::Done || t.status == TaskStatus::Failed);
            if all_terminal && !stage.tasks.is_empty() {
                plan.current_stage_index += 1;
                return true;
            }
        }
        false
    }

    /// Returns true when all stages are fully complete (all tasks done or failed).
    pub fn is_plan_complete(plan: &ProjectPlan) -> bool {
        plan.stages.iter().all(|stage| {
            !stage.tasks.is_empty()
                && stage
                    .tasks
                    .iter()
                    .all(|t| t.status == TaskStatus::Done || t.status == TaskStatus::Failed)
        })
    }
}

// ── Display helpers ───────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";

fn task_status_colored(status: &TaskStatus) -> String {
    match status {
        TaskStatus::Pending => format!("{DIM}pending{RESET}"),
        TaskStatus::InProgress => format!("{YELLOW}in_progress{RESET}"),
        TaskStatus::Done => format!("{GREEN}done{RESET}"),
        TaskStatus::Failed => format!("{RED}failed{RESET}"),
    }
}

pub fn print_plan(plan: &ProjectPlan) {
    println!("{BOLD}Project Plan{RESET}");
    println!("  Description: {}", plan.description);
    println!("  Repo:        {}", plan.repo_path);
    println!("  Model:       {}", plan.model);
    println!("  Max workers: {}", plan.max_concurrent_workers);
    println!(
        "  Stage:       {}/{}",
        plan.current_stage_index + 1,
        plan.stages.len()
    );
    println!();

    for (si, stage) in plan.stages.iter().enumerate() {
        let is_current = si == plan.current_stage_index;
        let marker = if is_current { "▶" } else { " " };
        println!(
            "{marker} {BOLD}Stage {} — {}{RESET}  {DIM}({}){RESET}",
            si, stage.name, stage.id
        );
        if !stage.description.is_empty() {
            println!("    {DIM}{}{RESET}", stage.description);
        }
        println!();

        for task in &stage.tasks {
            let status_str = task_status_colored(&task.status);
            let title_short: String = task.title.chars().take(60).collect();
            println!(
                "    [{status_str}]  {BOLD}{title_short}{RESET}  {DIM}({}){RESET}",
                task.id
            );
            if !task.description.is_empty() {
                // Print first 100 chars of description
                let desc_short: String = task.description.chars().take(100).collect();
                let ellipsis = if task.description.len() > 100 {
                    "…"
                } else {
                    ""
                };
                println!("             {DIM}{desc_short}{ellipsis}{RESET}");
            }
            if let Some(ref pane) = task.assigned_pane {
                println!("             {DIM}pane: {pane}{RESET}");
            }
        }
        println!();
    }

    if PlanManager::is_plan_complete(plan) {
        println!("{GREEN}{BOLD}Plan complete!{RESET}");
    } else if let Some(stage) = PlanManager::current_stage(plan) {
        let pending_count = stage
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .count();
        let in_progress_count = stage
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::InProgress)
            .count();
        println!(
            "{DIM}Current stage: \"{}\", {pending_count} pending, {in_progress_count} in progress{RESET}",
            stage.name
        );
    }
}
