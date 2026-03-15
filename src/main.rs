mod checkpoint;
mod events;
mod handlers;
mod harness;
mod health;
mod heartbeat;
mod layout;
mod loop_guard;
mod memory;
mod monitor;
mod output_cleaner;
mod pending_tasks;
mod project;
mod relay;
mod setup;
mod tasks;
mod tmux;
mod util;

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

        /// Strip TUI decorations, ANSI codes, and compact output for cleaner context
        #[arg(long)]
        clean: bool,
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

    /// Remove all done/cancelled tasks from the task list
    TaskCleanup,

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

    /// Internal: called every 1s by the hidden heartbeat daemon loop.
    /// Not intended for direct use — hidden from help output.
    #[command(hide = true)]
    HeartbeatDaemonTick,
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
            handlers::handle_init(&cli.dir, &bin)?;
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
            handlers::handle_spawn(task, dir, name, model, harness, mode, depends_on, no_hide)?;
        }

        Some(Command::Tasks) => {
            handlers::handle_tasks()?;
        }
        Some(Command::RunPending) => {
            handlers::handle_run_pending()?;
        }
        Some(Command::Read { pane, lines, clean }) => {
            handlers::handle_read(pane, lines, clean)?;
        }
        Some(Command::Send { pane, text }) => {
            handlers::handle_send(pane, text)?;
        }
        Some(Command::List) => {
            handlers::handle_list()?;
        }
        Some(Command::Kill { pane }) => {
            handlers::handle_kill(pane)?;
        }
        Some(Command::Hide { pane, name }) => {
            handlers::handle_hide(pane, name)?;
        }
        Some(Command::Show { pane, split }) => {
            handlers::handle_show(pane, split)?;
        }
        Some(Command::Surface { pane }) => {
            handlers::handle_surface(pane)?;
        }
        Some(Command::Compact) => {
            handlers::handle_compact()?;
        }
        Some(Command::Resize {
            pane,
            direction,
            amount,
        }) => {
            handlers::handle_resize(pane, direction, amount)?;
        }
        Some(Command::Layout { name }) => {
            handlers::handle_layout(name)?;
        }
        Some(Command::SmartLayout { hint }) => {
            handlers::handle_smart_layout(hint)?;
        }
        Some(Command::ToggleMode) => {
            handlers::handle_toggle_mode()?;
        }
        Some(Command::StatusHuman) => {
            handlers::handle_status_human()?;
        }
        Some(Command::Workers) => {
            handlers::handle_workers()?;
        }
        Some(Command::TerminalSize) => {
            handlers::handle_terminal_size()?;
        }
        Some(Command::Healthcheck { pane, interval }) => {
            handlers::handle_healthcheck(pane, interval)?;
        }
        Some(Command::LoopStatus { pane }) => {
            handlers::handle_loop_status(pane)?;
        }
        Some(Command::LoopClear { pane }) => {
            handlers::handle_loop_clear(pane)?;
        }
        Some(Command::Checkpoint { pane, note }) => {
            handlers::handle_checkpoint(pane, note)?;
        }
        Some(Command::Checkpoints { pane }) => {
            handlers::handle_checkpoints(pane)?;
        }
        Some(Command::Resume {
            checkpoint,
            dir,
            model,
        }) => {
            handlers::handle_resume(checkpoint, dir, model)?;
        }
        Some(Command::Ask { pane }) => {
            handlers::handle_ask(pane)?;
        }
        Some(Command::GitCheck { dir }) => {
            handlers::handle_git_check(dir)?;
        }
        Some(Command::Respawn {
            pane,
            task,
            dir,
            model,
            mode,
        }) => {
            handlers::handle_respawn(pane, task, dir, model, mode)?;
        }
        Some(Command::Memory {
            pane,
            key,
            value,
            list,
        }) => {
            handlers::handle_memory(pane, key, value, list)?;
        }

        // ── Task/subtask commands ────────────────────────────────────────────
        Some(Command::TaskAdd {
            title,
            description,
            priority,
            tags,
        }) => {
            handlers::handle_task_add(title, description, priority, tags)?;
        }
        Some(Command::TaskList { status, tag }) => {
            handlers::handle_task_list(status, tag)?;
        }
        Some(Command::TaskDone { id }) => {
            handlers::handle_task_done(id)?;
        }
        Some(Command::TaskStart { id }) => {
            handlers::handle_task_start(id)?;
        }
        Some(Command::TaskBlock { id }) => {
            handlers::handle_task_block(id)?;
        }
        Some(Command::TaskCancel { id }) => {
            handlers::handle_task_cancel(id)?;
        }
        Some(Command::TaskRemove { id }) => {
            handlers::handle_task_remove(id)?;
        }
        Some(Command::TaskShow { id }) => {
            handlers::handle_task_show(id)?;
        }
        Some(Command::TaskCleanup) => {
            handlers::handle_task_cleanup()?;
        }
        Some(Command::SubtaskAdd { task_id, title }) => {
            handlers::handle_subtask_add(task_id, title)?;
        }
        Some(Command::SubtaskDone {
            task_id,
            subtask_id,
        }) => {
            handlers::handle_subtask_done(task_id, subtask_id)?;
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
            handlers::handle_relay(pane, question, context, sensitive, wait_for, timeout)?;
        }
        Some(Command::RelayAnswer { id, answer }) => {
            handlers::handle_relay_answer(id, answer)?;
        }
        Some(Command::RelayList { pending }) => {
            handlers::handle_relay_list(pending)?;
        }
        Some(Command::SudoRelay {
            pane,
            command,
            execute,
            timeout,
        }) => {
            handlers::handle_sudo_relay(pane, command, execute, timeout)?;
        }
        Some(Command::SudoExec {
            pane,
            command,
            wait,
            timeout,
        }) => {
            handlers::handle_sudo_exec(pane, command, wait, timeout)?;
        }

        // ── Harness management ───────────────────────────────────────────────
        Some(Command::HarnessList) => {
            handlers::handle_harness_list()?;
        }
        Some(Command::HarnessSet { name }) => {
            handlers::handle_harness_set(name)?;
        }
        Some(Command::HarnessSwitch { name }) => {
            handlers::handle_harness_switch(name)?;
        }
        Some(Command::HarnessSettings) => {
            handlers::handle_harness_settings()?;
        }

        // ── Display commands ─────────────────────────────────────────────────
        Some(Command::EventFeed) => {
            handlers::handle_event_feed()?;
        }
        Some(Command::TasksModal) => {
            handlers::handle_tasks_modal()?;
        }
        Some(Command::StatusCounts) => {
            handlers::handle_status_counts()?;
        }

        // ── Heartbeat commands ───────────────────────────────────────────────
        Some(Command::Heartbeat { snooze }) => {
            handlers::handle_heartbeat(snooze)?;
        }
        Some(Command::HeartbeatToggle) => {
            handlers::handle_heartbeat_toggle()?;
        }
        Some(Command::HeartbeatStatus) => {
            handlers::handle_heartbeat_status()?;
        }
        Some(Command::HeartbeatDaemonTick) => {
            handlers::handle_heartbeat_daemon_tick()?;
        }
    }

    Ok(())
}
