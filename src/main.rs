mod checkpoint;
mod health;
mod loop_guard;
mod memory;
mod monitor;
mod pending_tasks;
mod setup;
mod state;
mod tmux;

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
    /// Spawn a new opencode worker as a pane
    Spawn {
        /// Task/prompt to give the worker
        #[arg(short, long)]
        task: String,

        /// Working directory for the worker
        #[arg(short, long, default_value = ".")]
        dir: String,

        /// Label/title for the pane (shown in pane border)
        #[arg(short, long)]
        name: Option<String>,

        /// Model to use (e.g. "fireworks/kimi-k2.5", "anthropic/claude-sonnet-4-6")
        #[arg(short, long)]
        model: Option<String>,

        /// Agent mode: build (default, full access) or plan (read-only planning)
        #[arg(long, default_value = "build")]
        mode: Option<String>,

        /// Comma-separated pane IDs that must finish before this worker starts (e.g. "%23,%24").
        /// When set, the task is written to pending_tasks.json and NOT spawned immediately.
        #[arg(long)]
        depends_on: Option<String>,
    },

    /// List all pending (dependency-gated) tasks
    Tasks,

    /// Check pending tasks and spawn any whose dependencies have all finished
    RunPending,

    /// Read recent output from a worker pane
    Read {
        /// Pane ID (from spawn/list output)
        #[arg(short, long)]
        pane: String,

        /// Number of lines to capture
        #[arg(short, long, default_value_t = 50)]
        lines: u32,
    },

    /// Send input/keystrokes to a worker pane
    Send {
        /// Pane ID
        #[arg(short, long)]
        pane: String,

        /// Text to send
        #[arg(short, long)]
        text: String,
    },

    /// List all panes in the superharness session
    List,

    /// Kill a worker pane
    Kill {
        /// Pane ID to kill
        #[arg(short, long)]
        pane: String,
    },

    /// Hide a pane to its own background tab
    Hide {
        /// Pane ID
        #[arg(short, long)]
        pane: String,

        /// Tab name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Surface a background pane back into the main window
    Show {
        /// Pane ID
        #[arg(short, long)]
        pane: String,

        /// Split direction: "h" (horizontal) or "v" (vertical)
        #[arg(short, long, default_value = "h")]
        split: String,
    },

    /// Resize a pane
    Resize {
        /// Pane ID
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
    },

    /// Return to present mode (human is back)
    Present,

    /// Show current mode and any pending decisions
    Status,

    /// Queue a decision for human review (useful in away mode)
    QueueDecision {
        /// Pane ID associated with this decision
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

    /// Monitor panes for stalls and auto-recover
    Monitor {
        /// Seconds between each check cycle
        #[arg(short, long, default_value_t = 60)]
        interval: u64,

        /// Specific pane ID to monitor (monitors all panes if omitted)
        #[arg(short, long)]
        pane: Option<String>,

        /// Number of consecutive unchanged checks before a pane is considered stalled
        #[arg(long, default_value_t = 3)]
        stall_threshold: u32,
    },

    /// One-shot health snapshot for pane(s) — returns structured JSON per pane
    Healthcheck {
        /// Specific pane ID to check (omit to check all panes)
        #[arg(short, long)]
        pane: Option<String>,

        /// Interval hint in seconds used to estimate last_activity_ago from stall counts
        /// (should match the interval you used when running monitor, defaults to 60)
        #[arg(short, long, default_value_t = 60)]
        interval: u64,
    },

    /// Show loop detection status for pane(s)
    LoopStatus {
        /// Pane ID to check (omit to check all panes)
        #[arg(short, long)]
        pane: Option<String>,
    },

    /// Clear loop history for a pane (after human breaks the loop)
    LoopClear {
        /// Pane ID to clear
        #[arg(short, long)]
        pane: String,
    },

    /// Save a checkpoint snapshot of a pane's output and metadata
    Checkpoint {
        /// Pane ID to snapshot
        #[arg(short, long)]
        pane: String,

        /// Optional human-readable note describing the checkpoint
        #[arg(short, long)]
        note: Option<String>,
    },

    /// List saved checkpoints
    Checkpoints {
        /// Filter to a specific pane ID (lists all panes if omitted)
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

    /// Store or list key-value memory facts for a pane
    Memory {
        /// Pane ID
        #[arg(short, long)]
        pane: String,

        /// Key to store (required when setting a value)
        #[arg(short, long)]
        key: Option<String>,

        /// Value to store (required when setting a value)
        #[arg(short = 'V', long)]
        value: Option<String>,

        /// List all stored memory entries for the pane
        #[arg(short, long)]
        list: bool,
    },
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
            tmux::init(&cli.dir)?;
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

        Some(Command::Away { message }) => {
            let sm = StateManager::new()?;
            sm.set_mode(state::Mode::Away, message.as_deref())?;
            let pending = sm.get_pending_decisions()?;
            let out = serde_json::json!({
                "mode": "away",
                "message": message,
                "pending_decisions": pending.len(),
                "checklist": [
                    "Should workers queue architecture decisions?",
                    "Should workers queue dependency/library choices?",
                    "Should workers queue breaking API changes?",
                    "Should workers queue security-sensitive operations?",
                    "Should workers queue destructive file operations?"
                ],
                "note": "Workers will queue critical decisions instead of auto-deciding. Run 'status' when you return."
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::Present) => {
            let sm = StateManager::new()?;
            let pending = sm.get_pending_decisions()?;
            sm.set_mode(state::Mode::Present, None)?;
            let out = serde_json::json!({
                "mode": "present",
                "pending_decisions": pending,
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

        Some(Command::QueueDecision {
            pane,
            question,
            context,
        }) => {
            let id = tmux::queue_decision(&pane, &question, &context)?;
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
    }

    Ok(())
}
