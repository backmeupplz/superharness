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
    Blocked,
    Cancelled,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::Blocked => write!(f, "blocked"),
            TaskStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Sort priority: lower number = shown first
impl TaskStatus {
    pub fn sort_order(&self) -> u8 {
        match self {
            TaskStatus::InProgress => 0,
            TaskStatus::Pending => 1,
            TaskStatus::Blocked => 2,
            TaskStatus::Done => 3,
            TaskStatus::Cancelled => 4,
        }
    }

    pub fn color_code(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "\x1b[33m",    // yellow
            TaskStatus::InProgress => "\x1b[32m", // green
            TaskStatus::Done => "\x1b[34m",       // blue
            TaskStatus::Blocked => "\x1b[31m",    // red
            TaskStatus::Cancelled => "\x1b[90m",  // dark grey
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::High => write!(f, "high"),
            Priority::Medium => write!(f, "medium"),
            Priority::Low => write!(f, "low"),
        }
    }
}

impl Priority {
    pub fn color_code(&self) -> &'static str {
        match self {
            Priority::High => "\x1b[31m",   // red
            Priority::Medium => "\x1b[33m", // yellow
            Priority::Low => "\x1b[36m",    // cyan
        }
    }
}

// ── Structs ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    pub done: bool,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: Option<Priority>,
    pub tags: Vec<String>,
    pub subtasks: Vec<Subtask>,
    pub created_at: u64,
    pub updated_at: u64,
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

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn gen_id() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ms = d.as_millis();
    // Use subsecond nanoseconds as a cheap pseudo-random 4-hex-char suffix
    let nanos = d.subsec_nanos();
    let suffix = format!("{:04x}", nanos & 0xFFFF);
    format!("{ms}{suffix}")
}

pub fn parse_priority(s: &str) -> Result<Priority> {
    match s.to_lowercase().as_str() {
        "high" | "h" => Ok(Priority::High),
        "medium" | "med" | "m" => Ok(Priority::Medium),
        "low" | "l" => Ok(Priority::Low),
        _ => anyhow::bail!("invalid priority '{}': use high, medium, or low", s),
    }
}

pub fn parse_status(s: &str) -> Result<TaskStatus> {
    match s.to_lowercase().replace('-', "_").as_str() {
        "pending" => Ok(TaskStatus::Pending),
        "in_progress" | "inprogress" | "started" | "wip" => Ok(TaskStatus::InProgress),
        "done" | "completed" | "finished" => Ok(TaskStatus::Done),
        "blocked" => Ok(TaskStatus::Blocked),
        "cancelled" | "canceled" => Ok(TaskStatus::Cancelled),
        _ => anyhow::bail!(
            "invalid status '{}': use pending, in_progress, done, blocked, or cancelled",
            s
        ),
    }
}

// ── TaskManager ───────────────────────────────────────────────────────────────

pub struct TaskManager {
    path: PathBuf,
}

impl TaskManager {
    pub fn new() -> Result<Self> {
        let dir = data_dir()?;
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create data directory: {}", dir.display()))?;
        let path = dir.join("tasks.json");
        Ok(Self { path })
    }

    pub fn load(&self) -> Result<Vec<Task>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read tasks file: {}", self.path.display()))?;
        let tasks: Vec<Task> = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse tasks file: {}", self.path.display()))?;
        Ok(tasks)
    }

    pub fn save(&self, tasks: &[Task]) -> Result<()> {
        let content = serde_json::to_string_pretty(tasks).context("failed to serialize tasks")?;
        fs::write(&self.path, &content)
            .with_context(|| format!("failed to write tasks file: {}", self.path.display()))?;
        Ok(())
    }

    pub fn add_task(
        &self,
        title: &str,
        description: Option<&str>,
        priority: Option<Priority>,
        tags: Vec<String>,
    ) -> Result<Task> {
        let mut tasks = self.load()?;
        let now = now_unix();
        let task = Task {
            id: gen_id(),
            title: title.to_string(),
            description: description.map(|s| s.to_string()),
            status: TaskStatus::Pending,
            priority,
            tags,
            subtasks: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        tasks.push(task.clone());
        self.save(&tasks)?;
        Ok(task)
    }

    pub fn list_tasks(
        &self,
        status_filter: Option<&str>,
        tag_filter: Option<&str>,
    ) -> Result<Vec<Task>> {
        let tasks = self.load()?;
        let mut filtered: Vec<Task> = tasks
            .into_iter()
            .filter(|t| {
                if let Some(sf) = status_filter {
                    if t.status.to_string() != sf {
                        return false;
                    }
                }
                if let Some(tf) = tag_filter {
                    if !t.tags.iter().any(|tag| tag == tf) {
                        return false;
                    }
                }
                true
            })
            .collect();
        // Sort: in_progress first, then pending, then blocked, done, cancelled
        filtered.sort_by_key(|t| t.status.sort_order());
        Ok(filtered)
    }

    /// Find a task by ID prefix (errors if 0 or multiple matches).
    pub fn get_task(&self, id_prefix: &str) -> Result<Task> {
        let tasks = self.load()?;
        let matches: Vec<&Task> = tasks
            .iter()
            .filter(|t| t.id.starts_with(id_prefix))
            .collect();
        match matches.len() {
            0 => anyhow::bail!("no task found with ID prefix: {}", id_prefix),
            1 => Ok(matches[0].clone()),
            _ => anyhow::bail!(
                "multiple tasks match prefix '{}' — use more characters",
                id_prefix
            ),
        }
    }

    pub fn set_status(&self, id_prefix: &str, status: TaskStatus) -> Result<Task> {
        let mut tasks = self.load()?;
        let indices: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.id.starts_with(id_prefix))
            .map(|(i, _)| i)
            .collect();
        match indices.len() {
            0 => anyhow::bail!("no task found with ID prefix: {}", id_prefix),
            1 => {
                let idx = indices[0];
                tasks[idx].status = status;
                tasks[idx].updated_at = now_unix();
                let updated = tasks[idx].clone();
                self.save(&tasks)?;
                Ok(updated)
            }
            _ => anyhow::bail!(
                "multiple tasks match prefix '{}' — use more characters",
                id_prefix
            ),
        }
    }

    pub fn add_subtask(&self, task_id_prefix: &str, title: &str) -> Result<Subtask> {
        let mut tasks = self.load()?;
        let indices: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.id.starts_with(task_id_prefix))
            .map(|(i, _)| i)
            .collect();
        match indices.len() {
            0 => anyhow::bail!("no task found with ID prefix: {}", task_id_prefix),
            1 => {
                let idx = indices[0];
                let subtask = Subtask {
                    id: gen_id(),
                    title: title.to_string(),
                    done: false,
                    created_at: now_unix(),
                };
                tasks[idx].subtasks.push(subtask.clone());
                tasks[idx].updated_at = now_unix();
                self.save(&tasks)?;
                Ok(subtask)
            }
            _ => anyhow::bail!(
                "multiple tasks match prefix '{}' — use more characters",
                task_id_prefix
            ),
        }
    }

    pub fn complete_subtask(&self, task_id_prefix: &str, subtask_id_prefix: &str) -> Result<()> {
        let mut tasks = self.load()?;
        let task_indices: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.id.starts_with(task_id_prefix))
            .map(|(i, _)| i)
            .collect();
        match task_indices.len() {
            0 => anyhow::bail!("no task found with ID prefix: {}", task_id_prefix),
            1 => {
                let task_idx = task_indices[0];
                let sub_indices: Vec<usize> = tasks[task_idx]
                    .subtasks
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.id.starts_with(subtask_id_prefix))
                    .map(|(i, _)| i)
                    .collect();
                match sub_indices.len() {
                    0 => anyhow::bail!("no subtask found with ID prefix: {}", subtask_id_prefix),
                    1 => {
                        let sub_idx = sub_indices[0];
                        tasks[task_idx].subtasks[sub_idx].done = true;
                        tasks[task_idx].updated_at = now_unix();
                        self.save(&tasks)?;
                        Ok(())
                    }
                    _ => anyhow::bail!(
                        "multiple subtasks match prefix '{}' — use more characters",
                        subtask_id_prefix
                    ),
                }
            }
            _ => anyhow::bail!(
                "multiple tasks match prefix '{}' — use more characters",
                task_id_prefix
            ),
        }
    }

    pub fn remove_task(&self, id_prefix: &str) -> Result<()> {
        let mut tasks = self.load()?;
        let indices: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.id.starts_with(id_prefix))
            .map(|(i, _)| i)
            .collect();
        match indices.len() {
            0 => anyhow::bail!("no task found with ID prefix: {}", id_prefix),
            1 => {
                tasks.remove(indices[0]);
                self.save(&tasks)?;
                Ok(())
            }
            _ => anyhow::bail!(
                "multiple tasks match prefix '{}' — use more characters",
                id_prefix
            ),
        }
    }
}

// ── Display helpers ───────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";

/// Print a human-readable task list table.
pub fn print_task_list(tasks: &[Task]) {
    if tasks.is_empty() {
        println!("No tasks found.");
        return;
    }

    // Header
    println!(
        "{}{:<8}  {:<12}  {:<8}  {:<40}  {}{:<10}{}",
        BOLD, "ID", "STATUS", "PRIORITY", "TITLE", "", "SUBTASKS", RESET
    );
    println!("{}", "─".repeat(86));

    for t in tasks {
        let id_short: String = t.id.chars().take(8).collect();
        let status_color = t.status.color_code();
        let status_str = t.status.to_string();
        let status_str_pad = format!("{:<12}", status_str);

        let priority_str = match &t.priority {
            Some(p) => {
                let pc = p.color_code();
                format!("{}{:<8}{}", pc, p.to_string(), RESET)
            }
            None => format!("{:<8}", ""),
        };

        let title_short: String = t.title.chars().take(40).collect();

        let done_count = t.subtasks.iter().filter(|s| s.done).count();
        let total = t.subtasks.len();
        let subtask_str = if total > 0 {
            format!("{}/{} done", done_count, total)
        } else {
            String::new()
        };

        println!(
            "{:<8}  {}{}{RESET}  {}  {:<40}  {:<10}",
            id_short, status_color, status_str_pad, priority_str, title_short, subtask_str,
        );
    }
}

/// Print a single task with full details.
pub fn print_task_detail(t: &Task) {
    let id_short: String = t.id.chars().take(8).collect();
    println!("{BOLD}Task: {RESET}{}", t.id);
    println!("  ID (short):  {id_short}");
    println!("  Title:       {}", t.title);
    if let Some(ref desc) = t.description {
        println!("  Description: {desc}");
    }
    let status_color = t.status.color_code();
    println!("  Status:      {}{}{RESET}", status_color, t.status);
    if let Some(ref p) = t.priority {
        println!("  Priority:    {}{}{RESET}", p.color_code(), p);
    }
    if !t.tags.is_empty() {
        println!("  Tags:        {}", t.tags.join(", "));
    }
    println!("  Created:     {}", t.created_at);
    println!("  Updated:     {}", t.updated_at);

    if t.subtasks.is_empty() {
        println!("  Subtasks:    none");
    } else {
        let done = t.subtasks.iter().filter(|s| s.done).count();
        println!("  Subtasks:    {}/{} done", done, t.subtasks.len());
        for s in &t.subtasks {
            let check = if s.done { "✓" } else { "○" };
            let sub_id: String = s.id.chars().take(8).collect();
            println!("    [{check}] {sub_id}  {}", s.title);
        }
    }
}
