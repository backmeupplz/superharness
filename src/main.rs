mod checkpoint;
mod events;
mod harness;
mod health;
mod layout;
mod loop_guard;
mod memory;
mod monitor;
mod pending_tasks;
mod project;
mod relay;
mod setup;
mod tasks;
mod tmux;
mod watch;

use anyhow::Context as _;
use clap::Parser;

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

        /// AI harness to use for this worker (opencode, claude, or codex).
        /// Overrides the configured default for this single spawn only.
        #[arg(long)]
        harness: Option<String>,

        /// Agent mode: build (default, full access) or plan (read-only planning)
        #[arg(long, default_value = "build")]
        mode: Option<String>,

        /// Comma-separated agent IDs that must finish before this worker starts (e.g. "%23,%24").
        /// When set, the task is written to pending_tasks.json and NOT spawned immediately.
        #[arg(long)]
        depends_on: Option<String>,

        /// Keep the spawned worker visible in the main orchestrator window.
        /// By default workers are immediately hidden to a background tab so the
        /// main window stays clean. Pass --no-hide to keep the pane visible instead.
        #[arg(long)]
        no_hide: bool,
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

    /// Apply the smart adaptive layout to the main window
    ///
    /// Optional hint controls the behaviour:
    ///   "maximize <pane_id>" — give that pane extra space and surface it
    ///   "focus <pane_id>"    — surface that pane then rebalance
    ///   "rebalance"          — standard smart rebalance (default)
    SmartLayout {
        /// Optional hint string (see above)
        #[arg(short = 'H', long)]
        hint: Option<String>,
    },

    /// Toggle between away and present mode by messaging the orchestrator
    ToggleMode,

    /// Show current mode and worker health in human-readable format (used by F3)
    StatusHuman,

    /// List active workers in human-readable format (used by F4)
    Workers,

    /// Report terminal dimensions and recommended worker layout (outputs JSON)
    TerminalSize,

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

    // ── Relay: worker-to-user credential/question relay ──────────────────────
    /// Workers call this to request input from the human (credentials, keys, etc.)
    Relay {
        /// Pane ID of the worker making the request (e.g. %5)
        #[arg(short, long)]
        pane: String,

        /// The question to ask the human
        #[arg(short, long)]
        question: String,

        /// Additional context explaining why this is needed
        #[arg(short, long, default_value = "")]
        context: String,

        /// Mark the answer as sensitive (passwords, keys — not echoed in logs)
        #[arg(long)]
        sensitive: bool,

        /// If set, block until this request ID is answered (or timeout expires).
        /// Workers use this to poll for an answer: --wait-for <id>
        #[arg(long)]
        wait_for: Option<String>,

        /// Timeout in seconds when using --wait-for (default 300 = 5 min)
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },

    /// Orchestrator calls this to answer a pending relay request
    RelayAnswer {
        /// Relay request ID (from relay-list output)
        #[arg(short, long)]
        id: String,

        /// The answer to provide
        #[arg(short, long)]
        answer: String,
    },

    /// List all relay requests (pending and answered)
    RelayList {
        /// Show only pending requests
        #[arg(long)]
        pending: bool,
    },

    /// Workers call this to relay a sudo command that needs a password
    SudoRelay {
        /// Pane ID of the worker
        #[arg(short, long)]
        pane: String,

        /// The command to run with sudo (without the 'sudo' prefix)
        #[arg(short, long)]
        command: String,

        /// If set, block until the relay is answered and execute the command.
        /// If not set, only creates the relay request and prints its ID.
        #[arg(long)]
        execute: bool,

        /// Timeout in seconds when using --execute (default 300)
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },

    /// Run a command with sudo directly (NOPASSWD path).
    /// Falls back to relay mechanism if sudo requires a password.
    SudoExec {
        /// Pane ID of the calling worker
        #[arg(short, long)]
        pane: String,

        /// The command to run with sudo (without the 'sudo' prefix)
        #[arg(short, long)]
        command: String,

        /// Block and wait for relay answer if a password is required (default true)
        #[arg(long, default_value_t = true)]
        wait: bool,

        /// Timeout in seconds when waiting for a relay answer (default 300)
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },

    // ── Harness management ───────────────────────────────────────────────────
    /// List detected AI harnesses and show which one is the current default
    HarnessList,

    /// Set the default AI harness written to ~/.config/superharness/config.json
    HarnessSet {
        /// Harness name: opencode, claude, or codex
        name: String,
    },

    /// Switch the active harness (errors if any workers are currently running)
    HarnessSwitch {
        /// Harness name to switch to: opencode, claude, or codex
        name: String,
    },

    /// Open an interactive settings popup: view current harness/model and switch harness
    ///
    /// Shows the configured default harness and model, then presents an arrow-key
    /// picker to change the default harness.  Writes the new choice to
    /// ~/.config/superharness/config.json.  Bound to F2 in the tmux status bar.
    HarnessSettings,

    /// Print the event log in human-readable colorized format (oldest-first, last 200 events)
    EventFeed,

    /// Show orchestrator tasks from .superharness/tasks.json grouped by status
    TasksModal,

    /// Print active/total worker count as "X/Y" for status bar display (e.g. "2/5").
    /// Active = workers whose output changed on the last monitor cycle (stall_count == 0).
    /// Total = all worker panes (excluding the orchestrator %0).
    StatusCounts,

    /// Immediately trigger a heartbeat check (workers call this when they finish).
    ///
    /// If the orchestrator is idle, sends [HEARTBEAT] to %0 right away without
    /// waiting for the 30-second watch-loop cooldown.
    ///
    /// If --snooze N is given, no heartbeat is sent — instead the snooze timer
    /// is set for N seconds (the orchestrator calls this to suppress heartbeats
    /// while it is busy).
    Heartbeat {
        /// Suppress heartbeats for the next N seconds (snooze mode).
        /// When set, no heartbeat is sent; only the snooze timer is updated.
        #[arg(long)]
        snooze: Option<u64>,
    },

    /// Print a short heartbeat status string for the tmux status bar.
    /// Reads heartbeat_state.json and emits an emoji + seconds-to-next-beat.
    HeartbeatStatus,

    /// Toggle heartbeat on/off. Called by clicking the heartbeat icon in the status bar.
    HeartbeatToggle,
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
            // Record active project directory
            let abs_dir = std::fs::canonicalize(&cli.dir)
                .unwrap_or_else(|_| std::path::PathBuf::from(&cli.dir));
            project::set_active_project(&abs_dir)?;

            // ── Fix 5: First-launch harness picker ──────────────────────────
            // If no default harness is configured, show an interactive picker
            // before the tmux session starts, so the user can choose.
            {
                let config_dir = dirs::config_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
                    .join("superharness");
                if harness::get_default_harness(&config_dir).is_none() {
                    let candidates = harness::detect_all_candidates();
                    if !candidates.is_empty() {
                        println!("Welcome to SuperHarness! Please select your default AI harness:");
                        println!();
                        match harness::run_interactive_picker(&candidates, None) {
                            Ok(Some(chosen)) => {
                                let _ = harness::set_default_harness(&config_dir, &chosen);
                                println!("  Default harness set to: {chosen}");
                            }
                            _ => {}
                        }
                        println!();
                    }
                }
            }

            // ── Fix 1: Auto-start watch daemon ───────────────────────────────
            // Spawn `superharness watch --interval 30` as a background daemon
            // so the heartbeat state file is kept up-to-date while the session
            // runs. A PID lock file prevents duplicate watch loops.
            {
                let data_dir = dirs::data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
                    .join("superharness");
                let _ = std::fs::create_dir_all(&data_dir);
                let pid_file = data_dir.join("watch.pid");

                let already_running = if let Ok(content) = std::fs::read_to_string(&pid_file) {
                    let pid: u32 = content.trim().parse().unwrap_or(0);
                    if pid > 0 {
                        // Check if the process is still alive via kill -0
                        std::process::Command::new("kill")
                            .args(["-0", &pid.to_string()])
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .map(|s| s.success())
                            .unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !already_running {
                    match std::process::Command::new(&bin)
                        .args(["watch", "--interval", "30"])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                    {
                        Ok(child) => {
                            let _ = std::fs::write(&pid_file, child.id().to_string());
                            // Drop child to detach; it continues after superharness exits.
                            drop(child);
                        }
                        Err(e) => {
                            eprintln!("warning: could not start watch daemon: {e}");
                        }
                    }
                }
            }

            setup::write_config(&cli.dir, &bin)?;
            tmux::init(&cli.dir, &bin)?;
        }
        Some(Command::Spawn {
            task,
            dir,
            name,
            model,
            harness,
            mode,
            depends_on,
            no_hide,
        }) => {
            if std::env::var("SUPERHARNESS_WORKER").is_ok() {
                eprintln!("error: workers cannot spawn sub-workers (SUPERHARNESS_WORKER is set)");
                std::process::exit(1);
            }

            if let Some(ref m) = mode {
                match m.as_str() {
                    "build" | "plan" => {}
                    other => anyhow::bail!(
                        "invalid mode {:?}: must be 'build' (default) or 'plan' (read-only planning)",
                        other
                    ),
                }
            }

            // Warn if the target dir is a git repo with uncommitted changes or
            // is in a state that can make worktrees tricky (detached HEAD, no commits).
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
                    // ── Detached HEAD check ──────────────────────────────────
                    // `git symbolic-ref --quiet HEAD` succeeds on a branch,
                    // fails when HEAD points directly at a commit (detached).
                    let is_detached = std::process::Command::new("git")
                        .args(["-C", &check_dir_str, "symbolic-ref", "--quiet", "HEAD"])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .map(|s| !s.success())
                        .unwrap_or(false);

                    if is_detached {
                        eprintln!("WARNING: {check_dir_str} is in detached HEAD state.");
                        eprintln!("  A worktree created from here will not be on any branch.");
                        eprintln!("  Consider checking out a branch first:");
                        eprintln!("    git -C {check_dir_str} checkout -b <branch-name>");
                    }

                    // ── Dirty-files check ────────────────────────────────────
                    let status_out = std::process::Command::new("git")
                        .args(["-C", &check_dir_str, "status", "--porcelain"])
                        .output();

                    if let Ok(out) = status_out {
                        let status_text = String::from_utf8_lossy(&out.stdout);
                        let dirty_lines: Vec<&str> = status_text
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .collect();
                        let dirty_count = dirty_lines.len();
                        if dirty_count > 0 {
                            // Categorise into staged, unstaged, and untracked for clarity.
                            let staged = dirty_lines
                                .iter()
                                .filter(|l| {
                                    let b = l.as_bytes();
                                    !b.is_empty() && b[0] != b' ' && b[0] != b'?'
                                })
                                .count();
                            let unstaged = dirty_lines
                                .iter()
                                .filter(|l| {
                                    let b = l.as_bytes();
                                    b.len() > 1 && b[1] != b' ' && b[0] == b' '
                                })
                                .count();
                            let untracked =
                                dirty_lines.iter().filter(|l| l.starts_with("??")).count();

                            eprintln!(
                                "WARNING: {check_dir_str} has {dirty_count} file(s) with uncommitted changes \
                                 ({staged} staged, {unstaged} unstaged, {untracked} untracked)."
                            );
                            eprintln!(
                                "  If you are using a git worktree, dirty files will NOT be included."
                            );
                            eprintln!("  Commit or stash them first, or run for details:");
                            eprintln!("    superharness git-check --dir {check_dir_str}");
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
                    harness.as_deref(),
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
                    harness.as_deref(),
                    mode.as_deref(),
                    no_hide,
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
                    t.harness.as_deref(),
                    t.mode.as_deref(),
                    false, // show in main window (default)
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

            // Trigger a heartbeat so the orchestrator wakes up immediately.
            let _ = watch::heartbeat();

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

        Some(Command::SmartLayout { hint }) => {
            let action = match hint.as_deref() {
                // "maximize <pane_id>" — give that pane extra space and surface it
                Some(h) if h.starts_with("maximize ") => {
                    let pane_id = h["maximize ".len()..].trim();
                    tmux::smart_layout_with_attention(Some(pane_id))?;
                    format!("maximized {pane_id}")
                }
                // "focus <pane_id>" — surface then rebalance
                Some(h) if h.starts_with("focus ") => {
                    let pane_id = h["focus ".len()..].trim();
                    tmux::surface(pane_id)?;
                    tmux::smart_layout()?;
                    format!("focused {pane_id}")
                }
                // "rebalance" or no hint — standard smart layout
                _ => {
                    tmux::smart_layout()?;
                    "rebalanced".to_string()
                }
            };
            let out = serde_json::json!({ "layout": "smart", "action": action, "hint": hint });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::ToggleMode) => {
            // Read project state to determine current mode
            let state_dir = project::get_project_state_dir()?;
            let state_file = state_dir.join("state.json");

            let current_mode = if state_file.exists() {
                let content = std::fs::read_to_string(&state_file).unwrap_or_default();
                let v: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
                v["mode"].as_str().unwrap_or("present").to_string()
            } else {
                "present".to_string()
            };

            // Find the main orchestrator pane (%0 or first pane)
            let panes = tmux::list().unwrap_or_default();
            let target_pane = panes
                .iter()
                .find(|p| p.id == "%0")
                .or_else(|| panes.first())
                .map(|p| p.id.clone())
                .unwrap_or_else(|| "%0".to_string());

            let (message, new_mode) = if current_mode == "away" {
                (
                    "The user has returned. Please read .superharness/state.json and .superharness/decisions.json to understand what happened while they were away, give them a brief natural-language debrief, then update .superharness/state.json to set mode to 'present' and clear away_since.",
                    "present",
                )
            } else {
                (
                    "The user wants to step away. Please ask them a few questions about what decisions you should queue vs auto-approve while they are gone, then update .superharness/state.json with {\"mode\": \"away\", \"away_since\": <unix_timestamp>, \"instructions\": <their preferences>} and adjust your behavior accordingly.",
                    "away",
                )
            };

            tmux::send(&target_pane, message)?;

            let out = serde_json::json!({
                "toggled": true,
                "previous_mode": current_mode,
                "requesting_mode": new_mode,
                "target_pane": target_pane,
                "note": "Message sent to orchestrator. The AI will handle the mode transition."
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

            // Read mode from project state file
            let state_dir = project::get_project_state_dir()?;
            let state_file = state_dir.join("state.json");
            let (mode_str, away_since, away_message) = if state_file.exists() {
                let content = std::fs::read_to_string(&state_file).unwrap_or_default();
                let v: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
                let mode = v["mode"].as_str().unwrap_or("present").to_string();
                let since = v["away_since"].as_u64();
                let msg = v["instructions"]
                    .as_str()
                    .or_else(|| v["away_message"].as_str())
                    .map(|s| s.to_string());
                (mode, since, msg)
            } else {
                ("present".to_string(), None, None)
            };

            // ── Hint bar ─────────────────────────────────────────────────────
            println!("  {DIM}any key to close{RESET}");
            println!("  {DIM}{}{RESET}", "─".repeat(70));

            // ── MODE ──────────────────────────────────────────────────────────
            println!();
            if mode_str == "away" {
                let away_since_str = away_since.map(|ts| {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let elapsed = now.saturating_sub(ts);
                    let h = elapsed / 3600;
                    let m = (elapsed % 3600) / 60;
                    format!("{h}h {m}m ago (since unix:{ts})")
                });
                println!("  {BOLD}{YELLOW}Mode:{RESET}    {BOLD}{YELLOW}AWAY{RESET}");
                if let Some(since) = away_since_str {
                    println!("  {DIM}Away:{RESET}    {since}");
                }
                if let Some(ref msg) = away_message {
                    println!("  {DIM}Message:{RESET} {msg}");
                }
            } else {
                println!("  {BOLD}{GREEN}Mode:{RESET}    {BOLD}{GREEN}PRESENT{RESET}");
            }

            // ── PENDING DECISIONS ─────────────────────────────────────────────
            let decisions_file = state_dir.join("decisions.json");
            println!();
            println!("  {BOLD}{UNDERLINE}Pending Decisions{RESET}");
            if decisions_file.exists() {
                let content = std::fs::read_to_string(&decisions_file).unwrap_or_default();
                let decisions: Vec<serde_json::Value> =
                    serde_json::from_str(&content).unwrap_or_default();
                if decisions.is_empty() {
                    println!("    {DIM}none{RESET}");
                } else {
                    println!("    {BOLD}{}{RESET} decision(s) queued", decisions.len());
                    for (i, d) in decisions.iter().enumerate() {
                        println!();
                        let pane = d["pane"].as_str().unwrap_or("?");
                        let question = d["question"].as_str().unwrap_or("?");
                        let context = d["context"].as_str().unwrap_or("");
                        println!("    {BOLD}[{}]{RESET} Agent {YELLOW}{}{RESET}", i + 1, pane);
                        println!("        {BOLD}Q:{RESET} {}", question);
                        if !context.is_empty() {
                            println!("        {DIM}Context:{RESET} {}", context);
                        }
                    }
                }
            } else {
                println!("    {DIM}none{RESET}");
            }

            // ── WORKER HEALTH ─────────────────────────────────────────────────
            println!();
            println!("  {BOLD}{UNDERLINE}Workers{RESET}");

            let monitor_state = monitor::load_state();
            let panes = tmux::list().unwrap_or_default();

            if panes.is_empty() {
                println!("    {DIM}(no workers running){RESET}");
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
                        "    {DIM}{}{RESET}  {status_colored}  {BOLD}{:<48}{RESET}{}",
                        p.id, short_title, attn
                    );
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

            // Hint bar
            println!("  {DIM}any key to close{RESET}");
            println!("  {DIM}{}{RESET}", "─".repeat(70));

            println!();
            if panes.is_empty() {
                println!("  {BOLD}Active Workers:{RESET} none");
                println!();
                println!("  {DIM}No workers currently running.{RESET}");
                println!(
                    "  {DIM}Spawn one with:{RESET} superharness spawn --task \"...\" --dir /path --model <model>"
                );
            } else {
                // Column widths: PANE 6, CMD 10, STATUS 8, TITLE 40, PATH 30
                const W_PANE: usize = 6;
                const W_CMD: usize = 10;
                const W_TITLE: usize = 40;
                const W_PATH: usize = 30;
                // total separator width
                let sep_width = W_PANE + 2 + W_CMD + 2 + W_TITLE + 2 + W_PATH;

                println!("  {BOLD}Active Workers:{RESET} {}", panes.len());
                println!();
                println!(
                    "  {BOLD}{UNDERLINE}{:<W_PANE$}  {:<W_CMD$}  {:<W_TITLE$}  {:<W_PATH$}{RESET}",
                    "AGENT", "CMD", "TITLE", "PATH"
                );
                println!("  {DIM}{}{RESET}", "─".repeat(sep_width));
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
                        "  {DIM}{:<W_PANE$}{RESET}  {CYAN}{:<W_CMD$}{RESET}  {BOLD}{:<W_TITLE$}{RESET}  {DIM}{:<W_PATH$}{RESET}",
                        p.id, short_cmd, short_title, short_path
                    );
                }
            }
            println!();
        }

        Some(Command::TerminalSize) => {
            let info = tmux::terminal_size_info();
            let out = serde_json::json!({
                "width": info.width,
                "height": info.height,
                "main_pane_rows": info.main_pane_rows,
                "workers_visible": info.workers_visible,
                "recommended_max_workers": info.recommended_max_workers,
            });
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
                None, // use default harness for resumed worker
                Some("build"),
                false, // show in main window (default)
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
            let new_pane = tmux::spawn(
                &retry_task,
                &dir,
                None,
                model.as_deref(),
                None, // use default harness for respawned worker
                mode.as_deref(),
                false, // show in main window (default)
            )?;

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

        // ── Relay commands ───────────────────────────────────────────────────
        Some(Command::Relay {
            pane,
            question,
            context,
            sensitive,
            wait_for,
            timeout,
        }) => {
            // If --wait-for is given, poll for the answer to an existing request.
            if let Some(ref req_id) = wait_for {
                match relay::wait_for_answer(req_id, timeout)? {
                    Some(answer) => {
                        let out = serde_json::json!({
                            "id": req_id,
                            "answered": true,
                            "answer": if sensitive { "<redacted>" } else { &answer },
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                        // Print the raw answer on its own line so workers can
                        // capture it with $(...) command substitution.
                        eprintln!("{answer}");
                    }
                    None => {
                        let out = serde_json::json!({
                            "id": req_id,
                            "answered": false,
                            "note": "timeout expired — no answer received",
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                        std::process::exit(1);
                    }
                }
            } else {
                // Create a new relay request.
                let id = relay::add_relay_request(&pane, &question, &context, sensitive)?;
                let out = serde_json::json!({
                    "id": id,
                    "pane": pane,
                    "question": question,
                    "context": context,
                    "sensitive": sensitive,
                    "status": "pending",
                    "note": format!(
                        "Relay request created. Poll for answer with: superharness relay --pane {pane} --question '' --wait-for {id}"
                    ),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
        }

        Some(Command::RelayAnswer { id, answer }) => {
            relay::answer_relay(&id, &answer)?;
            let out = serde_json::json!({
                "id": id,
                "answered": true,
                "note": "Answer stored. The worker will receive it on its next poll.",
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Some(Command::RelayList { pending }) => {
            let requests = if pending {
                relay::get_pending_relays()?
            } else {
                relay::list_all()?
            };

            let items: Vec<serde_json::Value> = requests
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "pane": r.pane_id,
                        "kind": r.kind.to_string(),
                        "question": r.question,
                        "context": r.context,
                        "sensitive": r.sensitive,
                        "status": r.status.to_string(),
                        // Never expose sensitive answers in list output.
                        "answer": if r.sensitive { r.answer.as_ref().map(|_| "<redacted>") } else { r.answer.as_deref() },
                        "created_at": r.created_at,
                        "answered_at": r.answered_at,
                    })
                })
                .collect();

            if requests.is_empty() {
                println!("No relay requests found.");
            } else {
                // Human-readable summary first.
                let pending_count = requests
                    .iter()
                    .filter(|r| r.status == relay::RelayStatus::Pending)
                    .count();
                println!(
                    "Relay requests: {} total, {} pending",
                    requests.len(),
                    pending_count
                );
                println!();

                for r in &requests {
                    let status_marker = match r.status {
                        relay::RelayStatus::Pending => "[PENDING ]",
                        relay::RelayStatus::Answered => "[answered]",
                        relay::RelayStatus::Cancelled => "[canceld ]",
                    };
                    let sens = if r.sensitive { " [sensitive]" } else { "" };
                    println!("{status_marker} {} (pane {}){}", r.id, r.pane_id, sens);
                    println!("  Q: {}", r.question);
                    if !r.context.is_empty() {
                        println!("  Context: {}", r.context);
                    }
                    if r.status == relay::RelayStatus::Pending {
                        println!(
                            "  Answer with: superharness relay-answer --id {} --answer \"<value>\"",
                            r.id
                        );
                    }
                    println!();
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "requests": items }))?
                );
            }
        }

        Some(Command::SudoRelay {
            pane,
            command,
            execute,
            timeout,
        }) => {
            let relay_id = relay::relay_sudo(&pane, &command)?;
            if execute {
                println!("Relay request {relay_id} created. Waiting for human to provide sudo password...");
                match relay::wait_for_answer(&relay_id, timeout)? {
                    Some(password) => {
                        let status = relay::run_sudo_with_password(&command, &password)?;
                        let out = serde_json::json!({
                            "relay_id": relay_id,
                            "command": command,
                            "exit_code": status.code(),
                            "success": status.success(),
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                        if !status.success() {
                            std::process::exit(status.code().unwrap_or(1));
                        }
                    }
                    None => {
                        let out = serde_json::json!({
                            "relay_id": relay_id,
                            "answered": false,
                            "note": "timeout expired — sudo password not provided",
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                        std::process::exit(1);
                    }
                }
            } else {
                let out = serde_json::json!({
                    "relay_id": relay_id,
                    "pane": pane,
                    "command": command,
                    "status": "pending",
                    "note": format!(
                        "Sudo relay created. Human should run: superharness relay-answer --id {relay_id} --answer \"<password>\""
                    ),
                    "poll_command": format!(
                        "superharness relay --pane {pane} --question '' --wait-for {relay_id}"
                    ),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
        }

        Some(Command::SudoExec {
            pane,
            command,
            wait,
            timeout,
        }) => {
            use relay::SudoExecResult;

            match relay::sudo_exec(&pane, &command)? {
                SudoExecResult::Success => {
                    let out = serde_json::json!({
                        "pane": pane,
                        "command": command,
                        "success": true,
                        "method": "nopasswd",
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                SudoExecResult::RelayCreated(relay_id) => {
                    if wait {
                        println!(
                            "sudo requires a password. Relay request {relay_id} created. Waiting for human..."
                        );
                        match relay::wait_for_answer(&relay_id, timeout)? {
                            Some(password) => {
                                let status = relay::run_sudo_with_password(&command, &password)?;
                                let out = serde_json::json!({
                                    "relay_id": relay_id,
                                    "pane": pane,
                                    "command": command,
                                    "exit_code": status.code(),
                                    "success": status.success(),
                                    "method": "relay_password",
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                                if !status.success() {
                                    std::process::exit(status.code().unwrap_or(1));
                                }
                            }
                            None => {
                                let out = serde_json::json!({
                                    "relay_id": relay_id,
                                    "answered": false,
                                    "note": "timeout expired — sudo password not provided",
                                });
                                println!("{}", serde_json::to_string_pretty(&out)?);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        let out = serde_json::json!({
                            "relay_id": relay_id,
                            "pane": pane,
                            "command": command,
                            "status": "awaiting_password",
                            "note": format!(
                                "sudo requires a password. Answer with: superharness relay-answer --id {relay_id} --answer \"<password>\""
                            ),
                        });
                        println!("{}", serde_json::to_string_pretty(&out)?);
                    }
                }
                SudoExecResult::Failed(msg) => {
                    let out = serde_json::json!({
                        "pane": pane,
                        "command": command,
                        "success": false,
                        "error": msg,
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                    std::process::exit(1);
                }
            }
        }

        // ── Harness management ───────────────────────────────────────────────
        Some(Command::HarnessList) => {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
                .join("superharness");

            let installed = harness::detect_installed();
            let default_name = harness::get_default_harness(&config_dir);

            if installed.is_empty() {
                println!("No AI harnesses detected on PATH.");
                println!();
                println!("Install one of the following:");
                println!("  opencode  (OpenCode)    — https://opencode.ai");
                println!("  claude    (Claude Code)  — https://claude.ai/code");
                println!("  codex     (OpenAI Codex) — https://github.com/openai/codex");
            } else {
                println!("Detected harnesses:");
                println!();
                for h in &installed {
                    // Determine if this is the current default
                    let is_default = default_name.as_deref() == Some(h.name.as_str())
                        || (default_name.is_none()
                            && installed.first().map(|f| f.name == h.name).unwrap_or(false));
                    let marker = if is_default { " *  (default)" } else { "" };
                    println!("  {:<10}  {}{}", h.binary, h.display_name, marker);
                }
                println!();
                if let Some(ref d) = default_name {
                    println!("Default (from config): {d}");
                } else {
                    println!("Default (auto-selected): {}", installed[0].binary);
                    println!("Set an explicit default with: superharness harness-set <name>");
                }
            }
        }

        Some(Command::HarnessSet { name }) => {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
                .join("superharness");

            // Validate: must be a known harness name
            let known = ["opencode", "claude", "codex"];
            if !known.contains(&name.as_str()) {
                anyhow::bail!(
                    "Unknown harness {:?}. Valid options: opencode, claude, codex",
                    name
                );
            }

            // Warn if the chosen harness is not actually installed
            let installed = harness::detect_installed();
            if !installed.iter().any(|h| h.name == name || h.binary == name) {
                eprintln!(
                    "WARNING: '{name}' does not appear to be installed on PATH.\n\
                     Install it from: {}",
                    harness::install_url(&name)
                );
            }

            harness::set_default_harness(&config_dir, &name)?;
            println!("Default harness set to: {name}");
        }

        Some(Command::HarnessSwitch { name }) => {
            // Refuse to switch if any worker panes are running
            let panes = tmux::list().unwrap_or_default();
            // %0 is the orchestrator — exclude it; any other pane is a worker
            let worker_panes: Vec<_> = panes.iter().filter(|p| p.id != "%0").collect();
            if !worker_panes.is_empty() {
                let ids: Vec<&str> = worker_panes.iter().map(|p| p.id.as_str()).collect();
                anyhow::bail!(
                    "Cannot switch harness while workers are running: {}.\n\
                     Kill all workers first with 'superharness kill --pane <id>', then retry.",
                    ids.join(", ")
                );
            }

            // Validate name
            let known = ["opencode", "claude", "codex"];
            if !known.contains(&name.as_str()) {
                anyhow::bail!(
                    "Unknown harness {:?}. Valid options: opencode, claude, codex",
                    name
                );
            }

            // Warn if not installed
            let installed = harness::detect_installed();
            if !installed.iter().any(|h| h.name == name || h.binary == name) {
                eprintln!(
                    "WARNING: '{name}' does not appear to be installed on PATH.\n\
                     Install it from: {}",
                    harness::install_url(&name)
                );
            }

            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
                .join("superharness");

            harness::set_default_harness(&config_dir, &name)?;
            println!("Harness switched to: {name}");
            println!("Workers spawned from now on will use '{name}'.");
        }

        Some(Command::HarnessSettings) => {
            use std::io::{self, Write};

            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
                .join("superharness");

            let current_harness = harness::get_default_harness(&config_dir);
            let current_model = harness::get_default_model(&config_dir);

            // ── Show current settings ────────────────────────────────────────
            println!();
            println!("  \x1b[1mSuperHarness Settings\x1b[0m");
            println!("  {}", "─".repeat(50));
            println!();
            let harness_display = current_harness.as_deref().unwrap_or("(auto-detected)");
            let model_display = current_model.as_deref().unwrap_or("(none set)");
            println!("  Current harness : \x1b[1;32m{harness_display}\x1b[0m");
            println!("  Current model   : \x1b[1;33m{model_display}\x1b[0m");
            println!();
            println!("  \x1b[2mChange harness (↑↓ move, Enter select, q cancel):\x1b[0m");
            println!();
            io::stdout().flush().ok();

            // Collect ALL candidates (installed or not) so user can see the full list.
            let candidates: Vec<harness::HarnessInfo> = harness::detect_all_candidates();

            match harness::run_interactive_picker(&candidates, current_harness.as_deref()) {
                Ok(Some(chosen)) => match harness::set_default_harness(&config_dir, &chosen) {
                    Ok(()) => {
                        println!(
                                "  \x1b[1;32m\u{2713}\x1b[0m Default harness set to: \x1b[1m{chosen}\x1b[0m"
                            );
                    }
                    Err(e) => {
                        eprintln!("  error: could not save config: {e}");
                    }
                },
                Ok(None) => {
                    println!("  No changes made.");
                }
                Err(e) => {
                    eprintln!("  picker error: {e}");
                }
            }
        }

        Some(Command::EventFeed) => {
            // ANSI helpers
            const RESET: &str = "\x1b[0m";
            const BOLD: &str = "\x1b[1m";
            const DIM: &str = "\x1b[2m";
            const GREEN: &str = "\x1b[32m";
            const RED: &str = "\x1b[31m";
            const YELLOW: &str = "\x1b[33m";
            const CYAN: &str = "\x1b[36m";

            let state_dir = project::get_project_state_dir()?;
            let events_path = state_dir.join("events.json");

            let all_events = events::load_events().unwrap_or_default();
            // Show last 200 events in chronological order (oldest first)
            let start = all_events.len().saturating_sub(200);
            let events = &all_events[start..];

            // Hint bar (first thing shown; q closes less, arrows scroll)
            println!("  {DIM}q:close  ↑/↓ or PgUp/PgDn:scroll  /:search{RESET}");
            println!("  {DIM}{}{RESET}", "─".repeat(70));

            println!();
            println!(
                "  {BOLD}Event Log:{RESET} {}  {DIM}({} total, showing last {}){RESET}",
                events_path.display(),
                all_events.len(),
                events.len()
            );
            println!();

            if events.is_empty() {
                println!("  {DIM}No events recorded yet.{RESET}");
            } else {
                for ev in events {
                    let secs = ev.timestamp;
                    let h = (secs % 86400) / 3600;
                    let m = (secs % 3600) / 60;
                    let s = secs % 60;
                    let time_str = format!("{h:02}:{m:02}:{s:02}");

                    let (color, kind_str) = match &ev.kind {
                        events::EventKind::WorkerSpawned => (GREEN, format!("{}", ev.kind)),
                        events::EventKind::WorkerKilled => (RED, format!("{}", ev.kind)),
                        events::EventKind::WorkerCompleted => (CYAN, format!("{}", ev.kind)),
                        events::EventKind::Pulse => (DIM, format!("{}", ev.kind)),
                        _ => (YELLOW, format!("{}", ev.kind)),
                    };

                    let pane_str = ev
                        .pane
                        .as_deref()
                        .map(|p| format!("  {DIM}{p}{RESET}"))
                        .unwrap_or_default();

                    let details = &ev.details;

                    // Print first line inline; indent any continuation lines so they
                    // don't appear flush against the left edge.
                    println!(
                        "  {DIM}[{time_str}]{RESET}  {color}{kind_str:<20}{RESET}{pane_str}  {}",
                        details.lines().next().unwrap_or("")
                    );
                    for cont_line in details.lines().skip(1) {
                        println!("    {DIM}{cont_line}{RESET}");
                    }
                }
            }
            println!();
        }

        Some(Command::TasksModal) => {
            // ANSI helpers
            const RESET: &str = "\x1b[0m";
            const BOLD: &str = "\x1b[1m";
            const DIM: &str = "\x1b[2m";
            const UNDERLINE: &str = "\x1b[4m";
            const GREEN: &str = "\x1b[32m";
            const RED: &str = "\x1b[31m";
            const YELLOW: &str = "\x1b[33m";

            #[derive(serde::Deserialize)]
            struct OrchestratorTask {
                id: String,
                title: String,
                #[serde(default)]
                description: String,
                status: String,
                #[serde(default)]
                priority: String,
                #[serde(default)]
                worker_pane: Option<String>,
            }

            let state_dir = project::get_project_state_dir()?;
            let tasks_path = state_dir.join("tasks.json");

            let tasks: Vec<OrchestratorTask> = if tasks_path.exists() {
                let content = std::fs::read_to_string(&tasks_path).unwrap_or_default();
                serde_json::from_str(&content).unwrap_or_default()
            } else {
                Vec::new()
            };

            // Count per status
            let count_in_progress = tasks.iter().filter(|t| t.status == "in-progress").count();
            let count_pending = tasks.iter().filter(|t| t.status == "pending").count();
            let count_blocked = tasks.iter().filter(|t| t.status == "blocked").count();
            let count_done = tasks.iter().filter(|t| t.status == "done").count();
            let count_cancelled = tasks.iter().filter(|t| t.status == "cancelled").count();

            // Hint bar (first thing shown; q closes less, arrows scroll)
            println!("  {DIM}q:close  ↑/↓ or PgUp/PgDn:scroll  /:search{RESET}");
            println!("  {DIM}{}{RESET}", "─".repeat(70));

            println!();
            println!(
                "  {BOLD}Tasks:{RESET} {}  {DIM}| in-progress:{} pending:{} blocked:{} done:{} cancelled:{}{RESET}",
                tasks.len(),
                count_in_progress,
                count_pending,
                count_blocked,
                count_done,
                count_cancelled,
            );
            println!("  {DIM}{}{RESET}", "─".repeat(72));
            println!();

            if tasks.is_empty() {
                println!("  {DIM}No tasks found in {}{RESET}", tasks_path.display());
                println!();
                // Done
            } else {
                // Order: in-progress, pending, blocked, done, cancelled
                let status_order = ["in-progress", "pending", "blocked", "done", "cancelled"];

                for status_key in &status_order {
                    let group: Vec<&OrchestratorTask> =
                        tasks.iter().filter(|t| t.status == *status_key).collect();
                    if group.is_empty() {
                        continue;
                    }

                    let (color, label) = match *status_key {
                        "in-progress" => (GREEN, "IN-PROGRESS"),
                        "pending" => (YELLOW, "PENDING"),
                        "blocked" => (RED, "BLOCKED"),
                        "done" => (DIM, "DONE"),
                        "cancelled" => (DIM, "CANCELLED"),
                        _ => ("\x1b[0m", *status_key),
                    };

                    println!("  {BOLD}{UNDERLINE}{color}{label}{RESET}");
                    println!();

                    for task in &group {
                        let priority_badge = match task.priority.as_str() {
                            "high" => format!("{RED}[HIGH]{RESET} "),
                            "medium" => format!("{YELLOW}[MED]{RESET}  "),
                            "low" => format!("{DIM}[LOW]{RESET}  "),
                            _ => String::new(),
                        };

                        let desc_preview: String = task.description.chars().take(80).collect();
                        let desc_suffix = if task.description.len() > 80 {
                            "…"
                        } else {
                            ""
                        };

                        let pane_str = task
                            .worker_pane
                            .as_deref()
                            .map(|p| format!("  {DIM}pane:{p}{RESET}"))
                            .unwrap_or_default();

                        println!(
                            "  {color}[{label}]{RESET} {priority_badge}{BOLD}{}{RESET}{pane_str}",
                            task.title
                        );
                        if !desc_preview.is_empty() {
                            println!("    {DIM}{}{}{RESET}", desc_preview, desc_suffix);
                        }
                        println!("    {DIM}id: {}{RESET}", task.id);
                        println!();
                    }
                }
            }
        }

        Some(Command::StatusCounts) => {
            println!("{}", watch::status_counts());
        }

        Some(Command::Heartbeat { snooze }) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if let Some(secs) = snooze {
                // Snooze mode: update snooze_until WITHOUT sending a heartbeat.
                let state = watch::read_heartbeat_state();
                let snooze_until = now + secs;
                watch::write_heartbeat_state_full(
                    state.last_beat_ts,
                    state.interval_secs,
                    state.last_sent,
                    state.needs_attention,
                    snooze_until,
                );
                eprintln!("[heartbeat] snoozed for {secs}s (until unix {snooze_until})");
            } else {
                // Immediate heartbeat: run idle checks and send if %0 is ready.
                // Respects snooze/toggle — does NOT clear it on success.
                match watch::heartbeat() {
                    Ok(true) => {
                        eprintln!("[heartbeat] sent [HEARTBEAT] to %0");
                    }
                    Ok(false) => {
                        eprintln!("[heartbeat] skipped — %0 is busy or snoozed");
                    }
                    Err(e) => {
                        eprintln!("[heartbeat] error: {e}");
                    }
                }
            }
        }

        Some(Command::HeartbeatToggle) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let state = watch::read_heartbeat_state();

            if state.snooze_until > now {
                // Currently snoozed — resume heartbeat by clearing the snooze.
                watch::write_heartbeat_state_full(
                    state.last_beat_ts,
                    state.interval_secs,
                    state.last_sent,
                    state.needs_attention,
                    0, // clear snooze
                );
                eprintln!("[heartbeat] toggled on (resumed)");
            } else {
                // Not snoozed — snooze for a very long time (~11 days = "off").
                let snooze_until = now + 999_999;
                watch::write_heartbeat_state_full(
                    state.last_beat_ts,
                    state.interval_secs,
                    state.last_sent,
                    state.needs_attention,
                    snooze_until,
                );
                eprintln!("[heartbeat] toggled off (snoozed)");
            }
        }

        Some(Command::HeartbeatStatus) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let state = watch::read_heartbeat_state();

            if state.last_beat_ts == 0 && state.snooze_until == 0 {
                // No heartbeat state file yet.
                print!("● --");
                return Ok(());
            }

            // Snooze takes priority in display.
            if state.snooze_until > now {
                let remaining = state.snooze_until - now;
                // Very long snooze (> 1 day) means the user toggled it off — show clean ‖.
                if remaining > 86400 {
                    print!("‖");
                } else {
                    print!("‖ {remaining}s");
                }
                return Ok(());
            }

            let secs_since_beat = now.saturating_sub(state.last_beat_ts);
            let secs_to_next = state.next_beat_ts.saturating_sub(now);

            let emoji = if secs_since_beat <= 3 {
                // Just fired.
                "◉"
            } else if !state.last_sent {
                // Last beat was skipped (busy).
                "○"
            } else if state.needs_attention {
                // Flashing: alternate every 5 seconds.
                if (now % 10) < 5 {
                    "●"
                } else {
                    "◉"
                }
            } else {
                "●"
            };

            print!("{emoji} {secs_to_next}s");
        }
    }

    Ok(())
}
