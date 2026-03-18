mod checkpoint;
mod events;
mod handlers;
mod harness;
mod health;
mod heartbeat;
mod layout;
mod memory;
mod monitor;
mod output_cleaner;
mod pending_tasks;
mod project;
mod setup;
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

        /// Return raw, unprocessed output (by default TUI decorations and ANSI codes are stripped)
        #[arg(long)]
        raw: bool,
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
        Some(Command::Read { pane, lines, raw }) => {
            handlers::handle_read(pane, lines, raw)?;
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
    }

    Ok(())
}
