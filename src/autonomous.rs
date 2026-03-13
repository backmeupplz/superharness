/// Autonomous execution engine — reads the project plan, spawns workers,
/// monitors them, and updates the plan while the user is away.
///
/// NOTE: The struct definitions here mirror the plan JSON format.
/// A parallel plan.rs worker is implementing the same structs — they will be
/// reconciled at merge. What matters is that both read/write the same JSON.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::events;
use crate::tmux;

// ── Plan structs (mirror of plan.rs — reconciled at merge) ─────────────────

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

// ── Plan file path ──────────────────────────────────────────────────────────

pub fn plan_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("superharness")
        .join("project_plan.json")
}

pub fn load_plan() -> Result<Option<ProjectPlan>> {
    let path = plan_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read plan file: {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let plan: ProjectPlan = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse plan file: {}", path.display()))?;
    Ok(Some(plan))
}

pub fn save_plan(plan: &ProjectPlan) -> Result<()> {
    let path = plan_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create plan directory: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(plan).context("failed to serialize plan")?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write plan file: {}", path.display()))?;
    Ok(())
}

// ── Plan summary helpers ────────────────────────────────────────────────────

pub struct PlanSummary {
    pub total_tasks: usize,
    pub done_tasks: usize,
    pub in_progress_tasks: usize,
    pub pending_tasks: usize,
    pub failed_tasks: usize,
    pub current_stage_name: String,
    pub total_stages: usize,
    pub current_stage_index: usize,
}

pub fn summarize_plan(plan: &ProjectPlan) -> PlanSummary {
    let all_tasks: Vec<&PlanTask> = plan.stages.iter().flat_map(|s| s.tasks.iter()).collect();
    let total_tasks = all_tasks.len();
    let done_tasks = all_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .count();
    let in_progress_tasks = all_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::InProgress)
        .count();
    let pending_tasks = all_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Pending)
        .count();
    let failed_tasks = all_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Failed)
        .count();

    let current_stage_name = plan
        .stages
        .get(plan.current_stage_index)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "complete".to_string());

    PlanSummary {
        total_tasks,
        done_tasks,
        in_progress_tasks,
        pending_tasks,
        failed_tasks,
        current_stage_name,
        total_stages: plan.stages.len(),
        current_stage_index: plan.current_stage_index,
    }
}

// ── Autonomous loop ─────────────────────────────────────────────────────────

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Create a git worktree for a task.  Returns the worktree path on success.
fn create_worktree(repo_path: &str, task_id: &str) -> Result<String> {
    let worktree_path = format!("/tmp/sh-{task_id}");

    let status = std::process::Command::new("git")
        .args(["-C", repo_path, "worktree", "add", &worktree_path, "HEAD"])
        .status()
        .with_context(|| {
            format!("failed to run git worktree add for task {task_id} in {repo_path}")
        })?;

    if !status.success() {
        anyhow::bail!(
            "git worktree add failed for task {task_id} (repo: {repo_path}, path: {worktree_path})"
        );
    }

    Ok(worktree_path)
}

/// Build the worker task prompt for an autonomous task.
fn build_worker_prompt(plan: &ProjectPlan, task: &PlanTask, worktree_path: &str) -> String {
    format!(
        "You are working in a git worktree at {worktree_path}. The main repo is at {repo_path}.\n\
        \n\
        Create a branch immediately: git checkout -b autonomous-{task_id}\n\
        Commit after every logical unit of work: git add -A && git commit -m 'wip: <description>'\n\
        \n\
        ## Task: {title}\n\
        \n\
        {description}\n\
        \n\
        When done: make sure all changes are committed. Run the build/tests if applicable.",
        worktree_path = worktree_path,
        repo_path = plan.repo_path,
        task_id = task.id,
        title = task.title,
        description = task.description,
    )
}

/// One iteration of the autonomous loop: reconcile panes, advance stage, spawn workers.
/// Returns true if the plan is fully complete (all stages done).
fn tick(plan: &mut ProjectPlan) -> Result<bool> {
    let active_panes: Vec<String> = tmux::list()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
        .collect();

    // ── Step 1: Reconcile in-progress tasks ──────────────────────────────────
    if let Some(stage) = plan.stages.get_mut(plan.current_stage_index) {
        for task in stage.tasks.iter_mut() {
            if task.status != TaskStatus::InProgress {
                continue;
            }
            let pane_id = match &task.assigned_pane {
                Some(p) => p.clone(),
                None => continue,
            };
            if !active_panes.contains(&pane_id) {
                // Pane is gone — mark task done
                println!(
                    "[AUTONOMOUS] Task completed (pane gone): {} ({})",
                    task.title, task.id
                );
                task.status = TaskStatus::Done;
                task.completed_at = Some(now_unix());
                let _ = events::log_event(
                    events::EventKind::WorkerKilled,
                    Some(&pane_id),
                    &format!("autonomous task completed: {}", task.title),
                );
            }
        }
    }

    // ── Step 2: Advance stage if complete ────────────────────────────────────
    loop {
        let stage_complete = match plan.stages.get(plan.current_stage_index) {
            None => break,
            Some(stage) => stage
                .tasks
                .iter()
                .all(|t| t.status == TaskStatus::Done || t.status == TaskStatus::Failed),
        };

        if !stage_complete {
            break;
        }

        plan.current_stage_index += 1;
        if plan.current_stage_index < plan.stages.len() {
            println!(
                "[AUTONOMOUS] Stage complete. Advancing to stage {}: {}",
                plan.current_stage_index,
                plan.stages
                    .get(plan.current_stage_index)
                    .map(|s| s.name.as_str())
                    .unwrap_or("?")
            );
            let _ = events::log_event(
                events::EventKind::ModeChanged,
                None,
                &format!("autonomous: advanced to stage {}", plan.current_stage_index),
            );
        }
    }

    // ── Step 3: Check if plan is complete ────────────────────────────────────
    if plan.current_stage_index >= plan.stages.len() {
        return Ok(true);
    }

    // ── Step 4: Spawn new workers for pending tasks ──────────────────────────
    // Count currently in-progress tasks across the current stage
    let in_progress_count = plan
        .stages
        .get(plan.current_stage_index)
        .map(|s| {
            s.tasks
                .iter()
                .filter(|t| t.status == TaskStatus::InProgress)
                .count()
        })
        .unwrap_or(0);

    let max = plan.max_concurrent_workers;
    let mut running = in_progress_count;

    // Collect indices of pending tasks in current stage (avoid borrow issues)
    let pending_indices: Vec<usize> = {
        let stage = &plan.stages[plan.current_stage_index];
        stage
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.status == TaskStatus::Pending)
            .map(|(i, _)| i)
            .collect()
    };

    for task_idx in pending_indices {
        if running >= max {
            break;
        }

        // Clone what we need before mutating
        let (task_id, task_title, task_description) = {
            let task = &plan.stages[plan.current_stage_index].tasks[task_idx];
            (
                task.id.clone(),
                task.title.clone(),
                task.description.clone(),
            )
        };

        println!(
            "[AUTONOMOUS] Spawning worker for: {} ({})",
            task_title, task_id
        );

        // Create git worktree
        let worktree_path = match create_worktree(&plan.repo_path, &task_id) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[AUTONOMOUS] WARNING: failed to create worktree for {task_id}: {e}");
                eprintln!("[AUTONOMOUS] Falling back to repo path for worker.");
                plan.repo_path.clone()
            }
        };

        // Build worker task prompt (need a temporary PlanTask for the helper)
        let prompt = {
            let tmp_task = PlanTask {
                id: task_id.clone(),
                title: task_title.clone(),
                description: task_description.clone(),
                status: TaskStatus::Pending,
                assigned_pane: None,
                worktree_path: None,
                started_at: None,
                completed_at: None,
            };
            build_worker_prompt(plan, &tmp_task, &worktree_path)
        };

        // Spawn the worker
        let pane_id = match tmux::spawn(
            &prompt,
            &worktree_path,
            Some(&task_id),
            Some(&plan.model),
            Some("build"),
        ) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("[AUTONOMOUS] ERROR: failed to spawn worker for {task_id}: {e}");
                continue;
            }
        };

        println!("[AUTONOMOUS] Worker spawned: {pane_id} for task {task_id}");
        let _ = events::log_event(
            events::EventKind::WorkerSpawned,
            Some(&pane_id),
            &format!("autonomous: {task_title}"),
        );

        // Update task
        let task = &mut plan.stages[plan.current_stage_index].tasks[task_idx];
        task.status = TaskStatus::InProgress;
        task.assigned_pane = Some(pane_id);
        task.worktree_path = Some(worktree_path);
        task.started_at = Some(now_unix());

        running += 1;
    }

    Ok(false)
}

/// Run the autonomous loop forever (until the plan completes or the process is killed).
pub fn run(interval_secs: u64) -> Result<()> {
    println!(
        "[AUTONOMOUS] Starting autonomous execution engine (interval={}s)",
        interval_secs
    );
    println!("[AUTONOMOUS] Plan file: {}", plan_file_path().display());
    println!("[AUTONOMOUS] Press Ctrl+C to stop.");
    println!();

    loop {
        println!("[AUTONOMOUS] Checking plan...");

        // Load fresh plan each iteration
        let plan_opt = match load_plan() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[AUTONOMOUS] ERROR reading plan: {e}");
                eprintln!("[AUTONOMOUS] Retrying in {interval_secs}s...");
                std::thread::sleep(Duration::from_secs(interval_secs));
                continue;
            }
        };

        let mut plan = match plan_opt {
            Some(p) => p,
            None => {
                println!(
                    "[AUTONOMOUS] No project plan found. Run: superharness plan \"description\""
                );
                println!("[AUTONOMOUS] Waiting {interval_secs}s...");
                std::thread::sleep(Duration::from_secs(interval_secs));
                continue;
            }
        };

        // Print current progress
        let summary = summarize_plan(&plan);
        println!(
            "[AUTONOMOUS] Plan: \"{}\" | Stage {}/{}: {} | Tasks: {}/{} done, {} in-progress, {} pending, {} failed",
            plan.description,
            summary.current_stage_index + 1,
            summary.total_stages,
            summary.current_stage_name,
            summary.done_tasks,
            summary.total_tasks,
            summary.in_progress_tasks,
            summary.pending_tasks,
            summary.failed_tasks,
        );

        // Run one tick
        let complete = match tick(&mut plan) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[AUTONOMOUS] ERROR in tick: {e}");
                false
            }
        };

        // Save updated plan
        if let Err(e) = save_plan(&plan) {
            eprintln!("[AUTONOMOUS] WARNING: failed to save plan: {e}");
        }

        if complete {
            println!();
            println!("[AUTONOMOUS] Plan complete! All stages done.");
            println!("[AUTONOMOUS] Entering idle monitoring (watching for new plan)...");
            println!();
            // Don't exit — watch for a new plan to appear
        }

        std::thread::sleep(Duration::from_secs(interval_secs));
    }
}
