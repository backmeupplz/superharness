mod autonomous;
mod checkpoint;
mod events;
mod health;
mod loop_guard;
mod memory;
mod monitor;
mod pending_tasks;
mod plan;
mod setup;
mod state;
mod tasks;
mod tmux;
mod watch;

use anyhow::Context as _;
use clap::Parser;
use state::StateManager;

#[derive(Parser)]
#[command(
    name = "superharness",
    about = "CLI tools for AI agent orchestration via tmux"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Working directory (for default init)
    #[arg(short, long, default_value = ".")]
    dir: String,

    /// Path to superharness binary (for dev mode)
    #[arg(long)]
    bin: Option<String>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Spawn a new opencode worker as an agent
    Spawn {
        /// Task/prompt to give the worker
        #[arg(short, long)]
        task: String,

        /// Working directory for the worker
        #[arg(short, long, default_value = ".")]
        dir: String,

        /// Label/title for the agent (shown in agent border)
        #[arg(short, long)]
        name: Option<String>,

        /// Model to use (e.g. "fireworks/kimi-k2.5", "anthropic/claude-sonnet-4-6")
        #[arg(short, long)]
        model: Option<String>,

        /// Agent mode: build (default, full access) or plan (read-only planning)
        #[arg(long, default_value = "build")]
        mode: Option<String>,

        /// Comma-separated agent IDs that must finish before this worker starts (e.g. "%23,%24").
        /// When set, the task is written to pending_tasks.json and NOT spawned immediately.
        #[arg(long)]
        depends_on: Option<String>,
    },

    /// List all pending (dependency-gated) tasks
    Tasks,

    /// Check pending tasks and spawn any whose dependencies have all finished
    RunPending,

    /// Read recent output from a worker agent
    Read {
        /// Agent ID (from spawn/list output)
        #[arg(short, long)]
        pane: String,

        /// Number of lines to capture
        #[arg(short, long, default_value_t = 50)]
        lines: u32,
    },

    /// Send input/keystrokes to a worker agent
    Send {
        /// Agent ID
        #[arg(short, long)]
        pane: String,

        /// Text to send
        #[arg(short, long)]
        text: String,
    },

    /// List all agents in the superharness session
    List,

    /// Kill a worker agent
    Kill {
        /// Agent ID to kill
        #[arg(short, long)]
        pane: String,
    },

    /// Hide an agent to its own background tab
    Hide {
        /// Agent ID
        #[arg(short, long)]
        pane: String,

        /// Tab name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Surface a background agent back into the main window
    Show {
        /// Agent ID
        #[arg(short, long)]
        pane: String,

        /// Split direction: "h" (horizontal) or "v" (vertical)
        #[arg(short, long, default_value = "h")]
        split: String,
    },

    /// Bring a background agent back into the main window with auto-layout (alias for show --split h)
    Surface {
        /// Agent ID to bring back into the main window
        #[arg(short, long)]
        pane: String,
    },

    /// Move small or excess agents to background tabs to keep the main window usable
    Compact,

    /// Resize an agent
    Resize {
        /// Agent ID
        #[arg(short, long)]
        pane: String,

        /// Direction: U, D, L, R
        #[arg(short, long)]
        direction: String,

        /// Number of cells
        #[arg(short, long, default_value_t = 10)]
        amount: u32,
    },

    /// Apply a layout preset to the session
    Layout {
        /// Layout name: tiled, main-vertical, main-horizontal, even-vertical, even-horizontal
        #[arg(short, long, default_value = "tiled")]
        name: String,
    },

    /// Enter away mode (human is not watching)
    Away {
        /// Optional message describing why you're going away or what to watch for
        #[arg(short, long)]
        message: Option<String>,

        /// Skip the interactive pre-authorization questions and use all defaults
        #[arg(short = 's', long)]
        skip_questions: bool,
    },

    /// Return to present mode (human is back)
    Present,

    /// Show current mode and any pending decisions
    Status,

    /// Show current mode, decisions, and worker health in human-readable format (used by F3)
    StatusHuman,

    /// List active workers in human-readable format (used by F4)
    Workers,

    /// Queue a decision for human review (useful in away mode)
    QueueDecision {
        /// Agent ID associated with this decision
        #[arg(short, long)]
        pane: String,

        /// The question or decision that needs human input
        #[arg(short, long)]
        question: String,

        /// Additional context to help the human decide
        #[arg(short, long, default_value = "")]
        context: String,
    },

    /// Clear all pending decisions
    ClearDecisions,

    /// Monitor agents for stalls and auto-recover
    Monitor {
        /// Seconds between each check cycle
        #[arg(short, long, default_value_t = 60)]
        interval: u64,

        /// Specific agent ID to monitor (monitors all agents if omitted)
        #[arg(short, long)]
        pane: Option<String>,

        /// Number of consecutive unchanged checks before an agent is considered stalled
        #[arg(long, default_value_t = 3)]
        stall_threshold: u32,
    },

    /// Auto follow-up and review loop: cleanup done agents, approve safe prompts, nudge stalled agents
    Watch {
        /// Seconds between each check cycle (default 30)
        #[arg(short, long, default_value_t = 30)]
        interval: u64,

        /// Specific agent ID to watch (watches all agents if omitted)
        #[arg(short, long)]
        pane: Option<String>,
    },

    /// Run the autonomous execution engine: reads the project plan, spawns workers, monitors them
    Autonomous {
        /// Seconds between each check cycle (default 30)
        #[arg(short, long, default_value_t = 30)]
        interval: u64,
    },

    /// Send a [PULSE] digest of all worker agents to the orchestrator agent (%0)
    Pulse,

    /// One-shot health snapshot for agent(s) — returns structured JSON per agent
    Healthcheck {
        /// Specific agent ID to check (omit to check all agents)
        #[arg(short, long)]
        pane: Option<String>,

        /// Interval hint in seconds used to estimate last_activity_ago from stall counts
        /// (should match the interval you used when running monitor, defaults to 60)
        #[arg(short, long, default_value_t = 60)]
        interval: u64,
    },

    /// Show loop detection status for agent(s)
    LoopStatus {
        /// Agent ID to check (omit to check all agents)
        #[arg(short, long)]
        pane: Option<String>,
    },

    /// Clear loop history for an agent (after human breaks the loop)
    LoopClear {
        /// Agent ID to clear
        #[arg(short, long)]
        pane: String,
    },

    /// Save a checkpoint snapshot of an agent's output and metadata
    Checkpoint {
        /// Agent ID to snapshot
        #[arg(short, long)]
        pane: String,

        /// Optional human-readable note describing the checkpoint
        #[arg(short, long)]
        note: Option<String>,
    },

    /// List saved checkpoints
    Checkpoints {
        /// Filter to a specific agent ID (lists all agents if omitted)
        #[arg(short, long)]
        pane: Option<String>,
    },

    /// Spawn a new worker that resumes from a saved checkpoint
    Resume {
        /// Checkpoint ID (from 'checkpoints' output, e.g. "%5/1741234567")
        #[arg(short, long)]
        checkpoint: String,

        /// Working directory for the new worker
        #[arg(short, long)]
        dir: String,

        /// Model to use (optional)
        #[arg(short, long)]
        model: Option<String>,
    },

    /// Store or list key-value memory facts for an agent
    Memory {
        /// Agent ID
        #[arg(short, long)]
        pane: String,

        /// Key to store (required when setting a value)
        #[arg(short, long)]
        key: Option<String>,

        /// Value to store (required when setting a value)
        #[arg(short = 'V', long)]
        value: Option<String>,

        /// List all stored memory entries for the agent
        #[arg(short, long)]
        list: bool,
    },

    /// Read last 20 lines of a worker agent and detect if it's asking a question
    Ask {
        /// Agent ID to inspect
        #[arg(short, long)]
        pane: String,
    },

    /// Check if a git repo directory has uncommitted changes before creating a worktree
    GitCheck {
        /// Directory containing the git repo to check
        #[arg(short, long, default_value = ".")]
        dir: String,
    },

    /// Kill a crashed worker and respawn it with crash context prepended to the task
    Respawn {
        /// Agent ID of the crashed worker
        #[arg(short, long)]
        pane: String,

        /// The original task to retry
        #[arg(short, long)]
        task: String,

        /// Working directory for the new worker
        #[arg(short, long)]
        dir: String,

        /// Model to use (optional)
        #[arg(short, long)]
        model: Option<String>,

        /// Agent mode: build (default) or plan
        #[arg(long, default_value = "build")]
        mode: Option<String>,
    },

    // ── Task/subtask storage ─────────────────────────────────────────────────
    /// Add a new task to the task list
    TaskAdd {
        /// Task title
        title: String,

        /// Optional description
        #[arg(short, long)]
        description: Option<String>,

        /// Priority: high, medium, or low
        #[arg(short, long)]
        priority: Option<String>,

        /// Comma-separated tags
        #[arg(short, long)]
        tags: Option<String>,
    },

    /// List tasks (with optional filters)
    TaskList {
        /// Filter by status: pending, in_progress, done, blocked, cancelled
        #[arg(short, long)]
        status: Option<String>,

        /// Filter by tag
        #[arg(short, long)]
        tag: Option<String>,
    },

    /// Mark a task as done
    TaskDone {
        /// Task ID prefix (at least 8 chars)
        id: String,
    },

    /// Mark a task as in_progress
    TaskStart {
        /// Task ID prefix
        id: String,
    },

    /// Mark a task as blocked
    TaskBlock {
        /// Task ID prefix
        id: String,
    },

    /// Mark a task as cancelled
    TaskCancel {
        /// Task ID prefix
        id: String,
    },

    /// Remove a task permanently
    TaskRemove {
        /// Task ID prefix
        id: String,
    },

    /// Show full details of a task
    TaskShow {
        /// Task ID prefix
        id: String,
    },

    /// Add a subtask to a task
    SubtaskAdd {
        /// Parent task ID prefix
        task_id: String,

        /// Subtask title
        title: String,
    },

    /// Mark a subtask as done
    SubtaskDone {
        /// Parent task ID prefix
        task_id: String,

        /// Subtask ID prefix
        subtask_id: String,
    },

    // ── Project Plan ────────────────────────────────────────────────────────
    /// Create a new AI-generated project plan and store it
    Plan {
        /// High-level description of what you want to build
        description: String,

        /// Path to the git repository (defaults to current directory)
        #[arg(short, long)]
        repo: Option<String>,

        /// Model to use for workers spawned to execute the plan
        #[arg(short, long, default_value = "anthropic/claude-sonnet-4-6")]
        model: String,

        /// Maximum number of concurrent workers when executing the plan
        #[arg(long, default_value_t = 3)]
        max_workers: usize,
    },

    /// Show the current project plan
    PlanShow,

    /// Reset all task statuses to pending (start over)
    PlanReset,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            let bin = cli.bin.unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.to_str().map(String::from))
                    .unwrap_or_else(|| "superharness".to_string())
            });
            setup::write_config(&cli.dir, &bin)?;
            tmux::init(&cli.dir, &bin)?;
        }
        Some(Command::Spawn {
            task,
            dir,
            name,
            model,
            mode,
            depends_on,
        }) => {
            if let Some(ref m) = mode {
                match m.as_str() {
                    "build" | "plan" => {}
                    other => anyhow::bail!(
                        "invalid mode {:?}: must be 'build' (default) or 'plan' (read-only planning)",
                        other
                    ),
                }
            }

            // Warn if the target dir is a git repo with uncommitted changes.
            // Worktrees are created from HEAD, so dirty files won't be included.
            {
                let check_dir =
                    std::fs::canonicalize(&dir).unwrap_or_else(|_| std::path::PathBuf::from(&dir));
                let check_dir_str = check_dir.to_string_lossy().to_string();

                let is_git = std::process::Command::new("git")
                    .args(["-C", &check_dir_str, "rev-parse", "--git-dir"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);

                if is_git {
                    let status_out = std::process::Command::new("git")
                        .args(["-C", &check_dir_str, "status", "--porcelain"])
                        .output();

                    if let Ok(out) = status_out {
                        let status_text = String::from_utf8_lossy(&out.stdout);
                        let dirty_count =
                            status_text.lines().filter(|l| !l.trim().is_empty()).count();
                        if dirty_count > 0 {
                            eprintln!(
                                "WARNING: {check_dir_str} has {dirty_count} uncommitted file(s)."
                            );
                            eprintln!(
                                "  If you are using a git worktree, dirty files will NOT be included."
                            );
                            eprintln!(
                                "  Run 'superharness git-check --dir {check_dir_str}' for details."
                            );
                        }
                    }
                }
            }

            // If --depends-on is provided, defer execution until dependencies finish.
            if let Some(deps_str) = depends_on {
                let deps: Vec<String> = deps_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let id = pending_tasks::add_task(
                    &task,
                    &dir,
                    model.as_deref(),
                    mode.as_deref(),
                    name.as_deref(),
                    deps.clone(),
                )?;
                let out = serde_json::json!({
                    "pending": true,
                    "task_id": id,
                    "depends_on": deps,
                    "note": "Task queued. Run 'run-pending' to spawn it once dependencies finish."
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                let pane = tmux::spawn(
                    &task,
                    &dir,
                    name.as_deref(),
                    model.as_deref(),
                    mode.as_deref(),
                )?;
                let short_task: String = task.chars().take(80).collect();
                let _ =
                    events::log_event(events::EventKind::WorkerSpawned, Some(&pane), &short_task);
                let out = serde_json::json!({ "pane": pane });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
        }

        Some(Command::Tasks) => {
            let tasks = pending_tasks::list_tasks()?;
            // Enrich each task with dependency status using current tmux pane list
            let active_panes: Vec<String> = tmux::list()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.id)
                .collect();
            let enriched: Vec<serde_json::Value> = tasks
                .iter()
                .map(|t| {
                    let deps_status: Vec<serde_json::Value> = t
                        .depends_on
                        .iter()
                        .map(|dep| {
                            serde_json::json!({
                                "pane": dep,
                                "done": !active_panes.contains(dep)
                            })
                        })
                        .collect();
                    let ready = deps_status
                        .iter()
                        .all(|d| d["done"].as_bool().unwrap_or(false));
                    serde_json::json!({
                        "id": t.id,
                        "task": t.task,
                        "dir": t.dir,
                        "model": t.model,
                        "mode": t.mode,
                        "name": t.name,
                        "depends_on": deps_status,
                        "ready": ready,
                        "created_at": t.created_at
                    })
                })
                .collect();
            let out = serde_json::json!({ "pending_tasks": enriched });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::RunPending) => {
            let active_panes: Vec<String> = tmux::list()
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.id)
                .collect();
            let ready = pending_tasks::ready_tasks(&active_panes)?;
            let mut spawned = Vec::new();
            for t in ready {
                match tmux::spawn(
                    &t.task,
                    &t.dir,
                    t.name.as_deref(),
                    t.model.as_deref(),
                    t.mode.as_deref(),
                ) {
                    Ok(pane_id) => {
                        pending_tasks::remove_task(&t.id)?;
                        spawned.push(serde_json::json!({
                            "task_id": t.id,
                            "pane": pane_id,
                            "task": t.task
                        }));
                    }
                    Err(e) => {
                        spawned.push(serde_json::json!({
                            "task_id": t.id,
                            "error": e.to_string(),
                            "task": t.task
                        }));
                    }
                }
            }
            let out = serde_json::json!({ "spawned": spawned });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Read { pane, lines }) => {
            let output = tmux::read(&pane, lines)?;
            let out = serde_json::json!({ "pane": pane, "output": output });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Send { pane, text }) => {
            tmux::send(&pane, &text)?;
            let out = serde_json::json!({ "pane": pane, "sent": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::List) => {
            let panes = tmux::list()?;
            let out = serde_json::json!({ "panes": panes });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Kill { pane }) => {
            tmux::kill(&pane)?;
            let _ = events::log_event(
                events::EventKind::WorkerKilled,
                Some(&pane),
                "worker killed",
            );
            let out = serde_json::json!({ "pane": pane, "killed": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Hide { pane, name }) => {
            tmux::hide(&pane, name.as_deref())?;
            let out = serde_json::json!({ "pane": pane, "hidden": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Show { pane, split }) => {
            tmux::show(&pane, &split)?;
            let out = serde_json::json!({ "pane": pane, "visible": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Surface { pane }) => {
            tmux::surface(&pane)?;
            let out = serde_json::json!({ "pane": pane, "visible": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Compact) => {
            let (moved, remaining) = tmux::compact_panes()?;
            let note = if moved > 0 {
                format!(
                    "{moved} agent(s) moved to background tabs. {remaining} agent(s) remain visible."
                )
            } else {
                "No agents needed moving — all agents meet size thresholds.".to_string()
            };
            let out = serde_json::json!({
                "moved_to_background": moved,
                "still_visible": remaining,
                "note": note,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Resize {
            pane,
            direction,
            amount,
        }) => {
            tmux::resize(&pane, &direction, amount)?;
            let out = serde_json::json!({ "pane": pane, "resized": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Some(Command::Layout { name }) => {
            tmux::layout(&name)?;
            let out = serde_json::json!({ "layout": name });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Away {
            message,
            skip_questions,
        }) => {
            use std::io::IsTerminal;
            use std::io::{self, BufRead, Write};

            let pre_auth = if !skip_questions && io::stdin().is_terminal() {
                // Helper: prompt a single [Y/n] question and return the boolean result.
                // An empty answer or 'y'/'Y' means "yes" (default); 'n'/'N' means "no".
                fn ask_yn(prompt: &str, default: bool) -> bool {
                    print!("{}", prompt);
                    let _ = io::stdout().flush();
                    let stdin = io::stdin();
                    let mut line = String::new();
                    if stdin.lock().read_line(&mut line).is_err() {
                        return default;
                    }
                    match line.trim().to_lowercase().as_str() {
                        "n" | "no" => false,
                        "y" | "yes" | "" => true,
                        _ => default,
                    }
                }

                println!();
                println!("Configure auto-approval for while you're away (Enter = keep default):");
                println!();

                let auto_approve_file_edits = ask_yn(
                    "  Auto-approve file edits in worker directories? [Y/n]: ",
                    true,
                );
                let auto_approve_git_commits = ask_yn(
                    "  Auto-approve git commits and branch operations? [Y/n]: ",
                    true,
                );
                let auto_approve_builds_tests =
                    ask_yn("  Auto-approve running builds and tests? [Y/n]: ", true);
                let flag_architecture_decisions = ask_yn(
                    "  Queue decisions about architecture/design choices? [Y/n]: ",
                    true,
                );
                let flag_destructive_operations = ask_yn(
                    "  Queue decisions about destructive operations (rm, force push)? [Y/n]: ",
                    true,
                );
                println!();

                state::PreAuth {
                    auto_approve_file_edits,
                    auto_approve_git_commits,
                    auto_approve_builds_tests,
                    flag_architecture_decisions,
                    flag_destructive_operations,
                }
            } else {
                // Not a TTY or --skip-questions: silently use all defaults.
                state::PreAuth::default()
            };

            let sm = StateManager::new()?;
            sm.set_mode(
                state::Mode::Away,
                message.as_deref(),
                Some(pre_auth.clone()),
            )?;
            let _ = events::log_event(events::EventKind::ModeChanged, None, "entered away mode");
            let pending = sm.get_pending_decisions()?;

            // Check if a project plan exists and build plan summary
            let plan_info = match autonomous::load_plan() {
                Ok(Some(plan)) => {
                    let summary = autonomous::summarize_plan(&plan);
                    serde_json::json!({
                        "found": true,
                        "description": plan.description,
                        "total_stages": summary.total_stages,
                        "total_tasks": summary.total_tasks,
                        "done_tasks": summary.done_tasks,
                        "in_progress_tasks": summary.in_progress_tasks,
                        "pending_tasks": summary.pending_tasks,
                        "current_stage": summary.current_stage_name,
                    })
                }
                _ => serde_json::json!({ "found": false }),
            };

            let autonomous_note = if plan_info["found"].as_bool().unwrap_or(false) {
                "Project plan found. Run 'superharness autonomous' in a separate pane to start autonomous execution."
            } else {
                "No project plan found. Create one with: superharness plan \"description\""
            };

            let out = serde_json::json!({
                "mode": "away",
                "message": message,
                "pending_decisions": pending.len(),
                "pre_auth": {
                    "auto_approve_file_edits": pre_auth.auto_approve_file_edits,
                    "auto_approve_git_commits": pre_auth.auto_approve_git_commits,
                    "auto_approve_builds_tests": pre_auth.auto_approve_builds_tests,
                    "flag_architecture_decisions": pre_auth.flag_architecture_decisions,
                    "flag_destructive_operations": pre_auth.flag_destructive_operations,
                },
                "project_plan": plan_info,
                "autonomous_mode": plan_info["found"].as_bool().unwrap_or(false),
                "autonomous_note": autonomous_note,
                "note": "Workers will use these pre-auth settings. Run 'status' when you return."
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Present) => {
            use std::time::{SystemTime, UNIX_EPOCH};

            let sm = StateManager::new()?;
            let current_state = sm.get_state()?;
            let away_since = current_state.away_since;
            let pending = sm.get_pending_decisions()?;
            sm.set_mode(state::Mode::Present, None, None)?;
            let _ = events::log_event(
                events::EventKind::ModeChanged,
                None,
                "returned to present mode",
            );

            // Build debrief if we were away
            let (debrief, away_duration_secs) = if let Some(since) = away_since {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let duration = now.saturating_sub(since);
                let ev = events::events_since(since).unwrap_or_default();
                let debrief_json: Vec<serde_json::Value> = ev
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "timestamp": e.timestamp,
                            "kind": e.kind.to_string(),
                            "pane": e.pane,
                            "details": e.details,
                        })
                    })
                    .collect();
                (debrief_json, Some(duration))
            } else {
                (Vec::new(), None)
            };

            // Build plan progress if a project plan exists
            let plan_progress = match autonomous::load_plan() {
                Ok(Some(plan)) => {
                    let summary = autonomous::summarize_plan(&plan);
                    serde_json::json!({
                        "found": true,
                        "description": plan.description,
                        "total_tasks": summary.total_tasks,
                        "done_tasks": summary.done_tasks,
                        "in_progress_tasks": summary.in_progress_tasks,
                        "pending_tasks": summary.pending_tasks,
                        "failed_tasks": summary.failed_tasks,
                        "current_stage": summary.current_stage_name,
                        "current_stage_index": summary.current_stage_index,
                        "total_stages": summary.total_stages,
                    })
                }
                _ => serde_json::json!({ "found": false }),
            };

            let out = serde_json::json!({
                "mode": "present",
                "away_duration_secs": away_duration_secs,
                "debrief": debrief,
                "pending_decisions": pending,
                "plan_progress": plan_progress,
                "note": if pending.is_empty() {
                    "No pending decisions. All clear!"
                } else {
                    "Review the pending decisions above. Use 'clear-decisions' after resolving them."
                }
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Status) => {
            let sm = StateManager::new()?;
            let s = sm.get_state()?;
            let out = serde_json::json!({
                "mode": s.mode.to_string(),
                "away_since": s.away_since,
                "away_message": s.away_message,
                "pending_decisions": s.pending_decisions,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::StatusHuman) => {
            use std::time::{SystemTime, UNIX_EPOCH};

            // ANSI helpers
            const RESET: &str = "\x1b[0m";
            const BOLD: &str = "\x1b[1m";
            const DIM: &str = "\x1b[2m";
            const UNDERLINE: &str = "\x1b[4m";
            const RED: &str = "\x1b[31m";
            const GREEN: &str = "\x1b[32m";
            const YELLOW: &str = "\x1b[33m";
            const BRIGHT_RED: &str = "\x1b[91m";
            const CYAN: &str = "\x1b[36m";

            let sm = StateManager::new()?;
            let s = sm.get_state()?;

            // ── MODE ──────────────────────────────────────────────────────────
            if matches!(s.mode, state::Mode::Away) {
                let away_since = s.away_since.map(|ts| {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let elapsed = now.saturating_sub(ts);
                    let h = elapsed / 3600;
                    let m = (elapsed % 3600) / 60;
                    format!("{h}h {m}m ago (since unix:{ts})")
                });
                println!("{BOLD}{YELLOW}Mode:{RESET}    {BOLD}{YELLOW}AWAY{RESET}");
                if let Some(since) = away_since {
                    println!("{DIM}Away:{RESET}    {since}");
                }
                if let Some(ref msg) = s.away_message {
                    println!("{DIM}Message:{RESET} {msg}");
                }
            } else {
                println!("{BOLD}{GREEN}Mode:{RESET}    {BOLD}{GREEN}PRESENT{RESET}");
            }

            // ── PENDING DECISIONS ─────────────────────────────────────────────
            println!();
            println!("{BOLD}{UNDERLINE}Pending Decisions{RESET}");
            if s.pending_decisions.is_empty() {
                println!("  {DIM}none{RESET}");
            } else {
                println!(
                    "  {BOLD}{}{RESET} decision(s) queued",
                    s.pending_decisions.len()
                );
                for (i, d) in s.pending_decisions.iter().enumerate() {
                    println!();
                    println!("  {BOLD}[{}]{RESET} Agent {YELLOW}{}{RESET}", i + 1, d.pane);
                    println!("      {BOLD}Q:{RESET} {}", d.question);
                    if !d.context.is_empty() {
                        println!("      {DIM}Context:{RESET} {}", d.context);
                    }
                }
            }

            // ── WORKER HEALTH ─────────────────────────────────────────────────
            println!();
            println!("{BOLD}{UNDERLINE}Workers{RESET}");

            let monitor_state = monitor::load_state();
            let panes = tmux::list().unwrap_or_default();

            if panes.is_empty() {
                println!("  {DIM}(no workers running){RESET}");
            } else {
                for p in &panes {
                    let health = health::classify_pane(&p.id, &monitor_state, 60).ok();
                    let (status_colored, status_plain) = match &health {
                        Some(h) => match h.status {
                            health::HealthStatus::Working => {
                                (format!("{DIM}{GREEN}working{RESET} "), "working ")
                            }
                            health::HealthStatus::Idle => {
                                (format!("{DIM}idle{RESET}    "), "idle    ")
                            }
                            health::HealthStatus::Stalled => {
                                (format!("{BOLD}{RED}STALLED{RESET} "), "STALLED ")
                            }
                            health::HealthStatus::Waiting => {
                                (format!("{BOLD}{YELLOW}WAITING{RESET} "), "WAITING ")
                            }
                            health::HealthStatus::Done => {
                                (format!("{DIM}done{RESET}    "), "done    ")
                            }
                        },
                        None => (format!("{DIM}unknown{RESET} "), "unknown "),
                    };
                    let _ = status_plain; // suppress unused warning
                    let attn = match &health {
                        Some(h) if h.needs_attention => {
                            format!("  {BOLD}{BRIGHT_RED}!! NEEDS ATTENTION{RESET}")
                        }
                        _ => String::new(),
                    };
                    let title = if p.title.is_empty() {
                        &p.command
                    } else {
                        &p.title
                    };
                    let short_title: String = title.chars().take(48).collect();
                    println!(
                        "  {DIM}{}{RESET}  {status_colored}  {BOLD}{:<48}{RESET}{}",
                        p.id, short_title, attn
                    );
                }
            }

            // ── DEBRIEF (only shown when away) ────────────────────────────────
            if matches!(s.mode, state::Mode::Away) {
                if let Some(since) = s.away_since {
                    let ev = events::events_since(since).unwrap_or_default();
                    println!();
                    println!("{BOLD}{UNDERLINE}Events Since Going Away{RESET}");
                    if ev.is_empty() {
                        println!("  {DIM}no events recorded yet{RESET}");
                    } else {
                        for e in &ev {
                            // Format timestamp as HH:MM:SS offset from `since`
                            let elapsed = e.timestamp.saturating_sub(since);
                            let h = elapsed / 3600;
                            let m = (elapsed % 3600) / 60;
                            let sec = elapsed % 60;
                            let ts_str = format!("+{h:02}:{m:02}:{sec:02}");
                            let pane_str = e
                                .pane
                                .as_deref()
                                .map(|p| format!(" {YELLOW}{p}{RESET}"))
                                .unwrap_or_default();
                            let kind_colored = match e.kind {
                                events::EventKind::WorkerSpawned => {
                                    format!("{GREEN}WorkerSpawned  {RESET}")
                                }
                                events::EventKind::WorkerKilled => {
                                    format!("{DIM}WorkerKilled   {RESET}")
                                }
                                events::EventKind::WorkerStalled => {
                                    format!("{BOLD}{RED}WorkerStalled  {RESET}")
                                }
                                events::EventKind::WorkerRecovered => {
                                    format!("{CYAN}WorkerRecovered{RESET}")
                                }
                                events::EventKind::DecisionQueued => {
                                    format!("{BOLD}{YELLOW}DecisionQueued {RESET}")
                                }
                                events::EventKind::DecisionCleared => {
                                    format!("{DIM}DecisionCleared{RESET}")
                                }
                                events::EventKind::ModeChanged => {
                                    format!("{DIM}ModeChanged    {RESET}")
                                }
                                events::EventKind::Pulse => {
                                    format!("{DIM}Pulse          {RESET}")
                                }
                            };
                            let short_details: String = e.details.chars().take(60).collect();
                            println!(
                                "  {DIM}{ts_str}{RESET}  {kind_colored}{pane_str}  {DIM}{short_details}{RESET}"
                            );
                        }
                    }
                }
            }

            println!();
        }

        Some(Command::Workers) => {
            // ANSI helpers
            const RESET: &str = "\x1b[0m";
            const BOLD: &str = "\x1b[1m";
            const DIM: &str = "\x1b[2m";
            const UNDERLINE: &str = "\x1b[4m";
            const CYAN: &str = "\x1b[36m";

            let panes = tmux::list().unwrap_or_default();

            // Abbreviate home directory in path
            let home = std::env::var("HOME").unwrap_or_default();
            let abbrev_path = |path: &str| -> String {
                if !home.is_empty() && path.starts_with(&home) {
                    format!("~{}", &path[home.len()..])
                } else {
                    path.to_string()
                }
            };

            if panes.is_empty() {
                println!("{BOLD}Active Workers:{RESET} none");
                println!();
                println!("{DIM}No workers currently running.{RESET}");
                println!(
                    "{DIM}Spawn one with:{RESET} superharness spawn --task \"...\" --dir /path --model <model>"
                );
            } else {
                // Column widths: PANE 6, CMD 10, STATUS 8, TITLE 40, PATH 30
                const W_PANE: usize = 6;
                const W_CMD: usize = 10;
                const W_TITLE: usize = 40;
                const W_PATH: usize = 30;
                // total separator width
                let sep_width = W_PANE + 2 + W_CMD + 2 + W_TITLE + 2 + W_PATH;

                println!("{BOLD}Active Workers:{RESET} {}", panes.len());
                println!();
                println!(
                    "{BOLD}{UNDERLINE}{:<W_PANE$}  {:<W_CMD$}  {:<W_TITLE$}  {:<W_PATH$}{RESET}",
                    "AGENT", "CMD", "TITLE", "PATH"
                );
                println!("{DIM}{}{RESET}", "─".repeat(sep_width));
                for p in &panes {
                    let title = if p.title.is_empty() {
                        &p.command
                    } else {
                        &p.title
                    };
                    let short_title: String = title.chars().take(W_TITLE).collect();
                    let path_abbrev = abbrev_path(&p.path);
                    let short_path: String = path_abbrev.chars().take(W_PATH).collect();
                    let short_cmd: String = p.command.chars().take(W_CMD).collect();
                    println!(
                        "{DIM}{:<W_PANE$}{RESET}  {CYAN}{:<W_CMD$}{RESET}  {BOLD}{:<W_TITLE$}{RESET}  {DIM}{:<W_PATH$}{RESET}",
                        p.id, short_cmd, short_title, short_path
                    );
                }
            }
        }

        Some(Command::QueueDecision {
            pane,
            question,
            context,
        }) => {
            let id = tmux::queue_decision(&pane, &question, &context)?;
            let short_q: String = question.chars().take(80).collect();
            let _ = events::log_event(events::EventKind::DecisionQueued, Some(&pane), &short_q);
            let out = serde_json::json!({
                "queued": true,
                "decision_id": id,
                "pane": pane,
                "question": question,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::ClearDecisions) => {
            let sm = StateManager::new()?;
            sm.clear_decisions()?;
            let _ = events::log_event(
                events::EventKind::DecisionCleared,
                None,
                "all pending decisions cleared",
            );
            let out = serde_json::json!({ "cleared": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Monitor {
            interval,
            pane,
            stall_threshold,
        }) => {
            monitor::run(interval, pane.as_deref(), stall_threshold)?;
        }

        Some(Command::Watch { interval, pane }) => {
            watch::run(interval, pane.as_deref())?;
        }

        Some(Command::Autonomous { interval }) => {
            autonomous::run(interval)?;
        }

        Some(Command::Pulse) => {
            let result = watch::pulse(true)?;
            let out = serde_json::json!({
                "sent": result.sent,
                "target_pane": result.target_pane,
                "message": result.message,
                "worker_count": result.worker_count,
                "reason_skipped": result.reason_skipped,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Healthcheck { pane, interval }) => {
            health::run(pane.as_deref(), interval)?;
        }

        Some(Command::LoopStatus { pane }) => match pane {
            Some(pane_id) => {
                let detection = loop_guard::get_loop_status(&pane_id)?;
                let out = serde_json::json!({
                    "pane": pane_id,
                    "loop_detected": detection.as_ref().map(|d| d.detected).unwrap_or(false),
                    "details": detection
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
            None => {
                let all_panes = loop_guard::get_all_panes()?;
                let mut results = Vec::new();
                for (pane_id, _count) in &all_panes {
                    let detection = loop_guard::get_loop_status(pane_id)?;
                    results.push(serde_json::json!({
                        "pane": pane_id,
                        "loop_detected": detection.as_ref().map(|d| d.detected).unwrap_or(false),
                        "details": detection
                    }));
                }
                let out = serde_json::json!({ "panes": results });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
        },

        Some(Command::LoopClear { pane }) => {
            loop_guard::clear_pane(&pane)?;
            let out = serde_json::json!({ "pane": pane, "cleared": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Checkpoint { pane, note }) => {
            // Capture current pane output (last 200 lines)
            let pane_output = tmux::read(&pane, 200)?;

            // Use the pane title as the task title; fall back to pane ID
            let pane_list = tmux::list()?;
            let task_title = pane_list
                .iter()
                .find(|p| p.id == pane)
                .map(|p| p.title.clone())
                .unwrap_or_else(|| pane.clone());

            let cp = checkpoint::save(&pane, &task_title, &pane_output, note.as_deref())?;
            let out = serde_json::json!({
                "checkpoint_id": cp.id,
                "pane": cp.pane_id,
                "timestamp": cp.timestamp,
                "task_title": cp.task_title,
                "note": cp.note,
                "lines_captured": cp.last_output.lines().count(),
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Checkpoints { pane }) => {
            let list = checkpoint::list(pane.as_deref())?;
            let out = serde_json::json!({ "checkpoints": list });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Resume {
            checkpoint,
            dir,
            model,
        }) => {
            let cp = checkpoint::load_by_id(&checkpoint)?;

            // Build a resume prompt containing context from the checkpoint
            let last_lines: String = cp
                .last_output
                .lines()
                .rev()
                .take(30)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            let resume_prompt = format!(
                "Resume this task. Previous context: {task_title}. \
                Last output was:\n{last_lines}\n\nContinue where it left off.",
                task_title = cp.task_title,
                last_lines = last_lines,
            );

            let note_suffix = cp
                .note
                .as_deref()
                .map(|n| format!(" (note: {n})"))
                .unwrap_or_default();
            let name = format!("resume of {}{}", cp.task_title, note_suffix);

            let pane_id = tmux::spawn(
                &resume_prompt,
                &dir,
                Some(&name),
                model.as_deref(),
                Some("build"),
            )?;
            let out = serde_json::json!({
                "pane": pane_id,
                "resumed_from": checkpoint,
                "task_title": cp.task_title,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Ask { pane }) => {
            let output = tmux::read(&pane, 20)?;
            let lines: Vec<&str> = output.lines().collect();

            // Patterns that suggest the worker is asking something
            let question_patterns: &[&str] = &[
                "?",
                "y/n",
                "Y/N",
                "yes/no",
                "Yes/No",
                "[y/n]",
                "[Y/N]",
                "Do you want",
                "Would you like",
                "Should I",
                "Can I",
                "Please confirm",
                "Enter ",
                "Provide ",
                "What ",
                "Which ",
                "How ",
                "Allow",
                "Approve",
                "Permission",
                "confirm",
                "proceed",
                "(y)",
                "(n)",
            ];

            // Find lines that look like questions or prompts
            let mut question_lines: Vec<(usize, &str)> = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                let lower = line.to_lowercase();
                let is_question = question_patterns
                    .iter()
                    .any(|p| line.contains(p) || lower.contains(&p.to_lowercase()));
                if is_question && !line.trim().is_empty() {
                    question_lines.push((i, line));
                }
            }

            println!("=== Agent {} — last {} lines ===", pane, lines.len());
            println!();
            for line in &lines {
                println!("  {line}");
            }
            println!();

            if question_lines.is_empty() {
                println!("[ No question or permission prompt detected ]");
                println!();
                println!("Worker appears to be working. Check back in 30-60s.");
            } else {
                println!("[ QUESTION / PROMPT DETECTED ]");
                println!();
                for (_, line) in &question_lines {
                    println!("  >> {line}");
                }
                println!();
                println!("To answer, run:");
                println!("  superharness send --pane {pane} --text \"<your answer>\"");
                println!();
                println!("To approve (yes):  superharness send --pane {pane} --text \"y\"");
                println!("To deny (no):      superharness send --pane {pane} --text \"n\"");
            }
        }

        Some(Command::GitCheck { dir }) => {
            let abs_dir =
                std::fs::canonicalize(&dir).with_context(|| format!("invalid directory: {dir}"))?;
            let dir_str = abs_dir.to_string_lossy().to_string();

            // Check if it's a git repo at all
            let is_git = std::process::Command::new("git")
                .args(["-C", &dir_str, "rev-parse", "--git-dir"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if !is_git {
                println!("Directory: {dir_str}");
                println!("Status:    NOT A GIT REPO");
                println!();
                println!("No git check needed — this directory is not a git repository.");
                println!("You can create worktrees only from git repos.");
            } else {
                // Run git status --porcelain to detect dirty files
                let status_out = std::process::Command::new("git")
                    .args(["-C", &dir_str, "status", "--porcelain"])
                    .output()
                    .with_context(|| "failed to run git status")?;

                let status_text = String::from_utf8_lossy(&status_out.stdout);
                let dirty_lines: Vec<&str> = status_text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .collect();

                println!("Directory: {dir_str}");

                if dirty_lines.is_empty() {
                    println!("Status:    CLEAN");
                    println!();
                    println!("Repo is clean. Safe to create a worktree from HEAD.");
                    println!();
                    println!("  git worktree add /tmp/worker-N HEAD");
                } else {
                    println!(
                        "Status:    DIRTY ({} file(s) with uncommitted changes)",
                        dirty_lines.len()
                    );
                    println!();
                    println!("Uncommitted changes:");
                    for line in &dirty_lines {
                        println!("  {line}");
                    }
                    println!();
                    println!("WARNING: Worktrees are created from HEAD. Dirty files will NOT");
                    println!("be included in the worktree. You should either:");
                    println!();
                    println!("  Option A — Commit your changes first:");
                    println!("    git add -A && git commit -m \"wip: save before worktree\"");
                    println!();
                    println!("  Option B — Stash your changes:");
                    println!(
                        "    git stash && git worktree add /tmp/worker-N HEAD && git stash pop"
                    );
                    println!();
                    println!("  Option C — Proceed anyway (dirty files stay in main only):");
                    println!("    git worktree add /tmp/worker-N HEAD");
                }
            }
        }

        Some(Command::Respawn {
            pane,
            task,
            dir,
            model,
            mode,
        }) => {
            // 1. Read last 100 lines for crash context
            let crash_context = tmux::read(&pane, 100)?;

            // 2. Kill the crashed pane
            tmux::kill(&pane)?;

            // 3. Build the retry task with crash context prepended
            let retry_task = format!(
                "Previous attempt crashed. Context from crash:\n{crash_context}\n\nPlease retry the task, avoiding whatever caused the crash.\n\nOriginal task: {task}"
            );

            // 4. Spawn a new worker
            let new_pane = tmux::spawn(&retry_task, &dir, None, model.as_deref(), mode.as_deref())?;

            println!("Crashed agent {} killed.", pane);
            println!("New worker spawned: {new_pane}");
            println!();
            println!("The new worker has been given the crash context and will retry the task.");
            println!("Monitor with: superharness read --pane {new_pane} --lines 50");
        }

        Some(Command::Memory {
            pane,
            key,
            value,
            list,
        }) => {
            if list {
                let mem = memory::get_all(&pane)?;
                let out = serde_json::json!({
                    "pane": mem.pane_id,
                    "memory": mem.entries,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                match (key, value) {
                    (Some(k), Some(v)) => {
                        memory::set(&pane, &k, &v)?;
                        let out = serde_json::json!({
                            "pane": pane,
                            "stored": true,
                            "key": k,
                            "value": v,
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                    }
                    _ => {
                        anyhow::bail!(
                            "provide --key and --value to store a fact, or --list to retrieve all"
                        );
                    }
                }
            }
        }

        // ── Task/subtask commands ────────────────────────────────────────────
        Some(Command::TaskAdd {
            title,
            description,
            priority,
            tags,
        }) => {
            let tm = tasks::TaskManager::new()?;
            let priority = priority.as_deref().map(tasks::parse_priority).transpose()?;
            let tag_list: Vec<String> = tags
                .as_deref()
                .unwrap_or("")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let task = tm.add_task(&title, description.as_deref(), priority, tag_list)?;
            let id_short: String = task.id.chars().take(8).collect();
            println!("Task created: {id_short}  \"{}\"", task.title);
            println!("Full ID: {}", task.id);
            println!();
            println!("Reference this task with any unique prefix of its ID (e.g. '{id_short}').");
        }

        Some(Command::TaskList { status, tag }) => {
            let tm = tasks::TaskManager::new()?;
            let status_filter = status
                .as_deref()
                .map(tasks::parse_status)
                .transpose()?
                .map(|s| s.to_string());
            let task_list = tm.list_tasks(status_filter.as_deref(), tag.as_deref())?;
            tasks::print_task_list(&task_list);
        }

        Some(Command::TaskDone { id }) => {
            let tm = tasks::TaskManager::new()?;
            let task = tm.set_status(&id, tasks::TaskStatus::Done)?;
            let id_short: String = task.id.chars().take(8).collect();
            println!("Task {id_short} marked as done: \"{}\"", task.title);
        }

        Some(Command::TaskStart { id }) => {
            let tm = tasks::TaskManager::new()?;
            let task = tm.set_status(&id, tasks::TaskStatus::InProgress)?;
            let id_short: String = task.id.chars().take(8).collect();
            println!("Task {id_short} marked as in_progress: \"{}\"", task.title);
        }

        Some(Command::TaskBlock { id }) => {
            let tm = tasks::TaskManager::new()?;
            let task = tm.set_status(&id, tasks::TaskStatus::Blocked)?;
            let id_short: String = task.id.chars().take(8).collect();
            println!("Task {id_short} marked as blocked: \"{}\"", task.title);
        }

        Some(Command::TaskCancel { id }) => {
            let tm = tasks::TaskManager::new()?;
            let task = tm.set_status(&id, tasks::TaskStatus::Cancelled)?;
            let id_short: String = task.id.chars().take(8).collect();
            println!("Task {id_short} marked as cancelled: \"{}\"", task.title);
        }

        Some(Command::TaskRemove { id }) => {
            let tm = tasks::TaskManager::new()?;
            tm.remove_task(&id)?;
            println!("Task removed.");
        }

        Some(Command::TaskShow { id }) => {
            let tm = tasks::TaskManager::new()?;
            let task = tm.get_task(&id)?;
            tasks::print_task_detail(&task);
        }

        Some(Command::SubtaskAdd { task_id, title }) => {
            let tm = tasks::TaskManager::new()?;
            let subtask = tm.add_subtask(&task_id, &title)?;
            let sub_id_short: String = subtask.id.chars().take(8).collect();
            println!("Subtask created: {sub_id_short}  \"{}\"", subtask.title);
        }

        Some(Command::SubtaskDone {
            task_id,
            subtask_id,
        }) => {
            let tm = tasks::TaskManager::new()?;
            tm.complete_subtask(&task_id, &subtask_id)?;
            println!("Subtask marked as done.");
        }

        // ── Project Plan commands ────────────────────────────────────────────
        Some(Command::Plan {
            description,
            repo,
            model,
            max_workers,
        }) => {
            use std::time::Duration;

            let repo_path = match repo {
                Some(r) => std::fs::canonicalize(&r)
                    .with_context(|| format!("invalid repo path: {r}"))?
                    .to_string_lossy()
                    .to_string(),
                None => std::env::current_dir()
                    .context("failed to get current directory")?
                    .to_string_lossy()
                    .to_string(),
            };

            let pm = plan::PlanManager::new()?;
            let plan_file = plan::plan_file_path()?;
            let plan_file_str = plan_file.to_string_lossy().to_string();

            // Build the planner prompt
            let schema_example = r#"{
  "description": "...",
  "repo_path": "...",
  "model": "...",
  "max_concurrent_workers": 3,
  "current_stage_index": 0,
  "stages": [
    {
      "id": "stage-0",
      "name": "...",
      "description": "...",
      "tasks": [
        {
          "id": "task-0-0",
          "title": "...",
          "description": "Detailed description for the AI worker",
          "status": "pending",
          "assigned_pane": null,
          "worktree_path": null,
          "started_at": null,
          "completed_at": null
        }
      ]
    }
  ]
}"#;

            let planner_task = format!(
                "You are a software project planner. Analyze this project and create a detailed, staged implementation plan.\n\n\
                Project description: {description}\n\
                Git repository: {repo_path}\n\n\
                Your job: Write a project plan as JSON to this exact file path: {plan_file_str}\n\n\
                The JSON must match this exact schema:\n{schema_example}\n\n\
                Requirements:\n\
                - 2-4 stages, ordered from foundations to features to polish\n\
                - 3-5 tasks per stage\n\
                - Each task description must be detailed enough that an AI coding agent can implement it independently, with no ambiguity\n\
                - Tasks within a stage can run in parallel (they should not depend on each other)\n\
                - model: \"{model}\"\n\
                - max_concurrent_workers: {max_workers}\n\
                - repo_path: \"{repo_path}\"\n\
                - current_stage_index: 0\n\
                - All task statuses start as \"pending\"\n\
                - Generate unique IDs like \"stage-0\", \"stage-1\", \"task-0-0\", \"task-0-1\", etc.\n\n\
                Write ONLY the JSON file. Do not start any implementation. Output a brief summary of the plan after writing the file.",
                description = description,
                repo_path = repo_path,
                plan_file_str = plan_file_str,
                schema_example = schema_example,
                model = model,
                max_workers = max_workers,
            );

            println!("Creating project plan...");
            println!("  Description: {description}");
            println!("  Repo:        {repo_path}");
            println!("  Model:       {model}");
            println!("  Max workers: {max_workers}");
            println!();
            println!("Spawning planner agent...");

            let pane_id = tmux::spawn(
                &planner_task,
                &repo_path,
                Some("planner"),
                Some(&model),
                Some("plan"),
            )?;

            println!("Planner agent spawned: {pane_id}");
            println!("Waiting for plan file to be written (up to 5 minutes)...");

            // Poll every 5s for up to 5 minutes (60 attempts)
            let mut plan_ready = false;
            for attempt in 0..60usize {
                std::thread::sleep(Duration::from_secs(5));

                // Check if file exists and is valid JSON
                if plan_file.exists() {
                    match std::fs::read_to_string(&plan_file) {
                        Ok(content) => {
                            if serde_json::from_str::<plan::ProjectPlan>(&content).is_ok() {
                                plan_ready = true;
                                break;
                            }
                        }
                        Err(_) => {}
                    }
                }

                if attempt % 6 == 5 {
                    let elapsed = (attempt + 1) * 5;
                    println!("  Still waiting... ({elapsed}s elapsed)");
                }
            }

            // Kill the planner pane
            let _ = tmux::kill(&pane_id);

            if !plan_ready {
                anyhow::bail!(
                    "Planner did not produce a valid plan file within 5 minutes.\n\
                    Check if the planner output any errors, or try again."
                );
            }

            // Load and display
            match pm.load()? {
                Some(p) => {
                    println!();
                    println!("Plan created successfully!");
                    println!();
                    plan::print_plan(&p);
                    println!();
                    println!("Run 'superharness plan-show' to view the plan at any time.");
                }
                None => {
                    anyhow::bail!("Plan file was written but could not be loaded.");
                }
            }
        }

        Some(Command::PlanShow) => {
            let pm = plan::PlanManager::new()?;
            match pm.load()? {
                Some(p) => plan::print_plan(&p),
                None => {
                    println!("No project plan found.");
                    println!();
                    println!("Create one with:");
                    println!("  superharness plan \"description of what you want to build\" --repo /path/to/repo");
                }
            }
        }

        Some(Command::PlanReset) => {
            let pm = plan::PlanManager::new()?;
            match pm.load()? {
                None => {
                    println!("No project plan found. Nothing to reset.");
                }
                Some(mut p) => {
                    p.current_stage_index = 0;
                    for stage in &mut p.stages {
                        for task in &mut stage.tasks {
                            task.status = plan::TaskStatus::Pending;
                            task.assigned_pane = None;
                            task.worktree_path = None;
                            task.started_at = None;
                            task.completed_at = None;
                        }
                    }
                    pm.save(&p)?;
                    println!("Plan reset: all tasks set to pending, stage index reset to 0.");
                    println!();
                    plan::print_plan(&p);
                }
            }
        }
    }

    Ok(())
}
