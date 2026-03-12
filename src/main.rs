mod loop_guard;
mod setup;
mod tmux;

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
    /// Spawn a new opencode worker as a pane
    Spawn {
        /// Task/prompt to give the worker
        #[arg(short, long)]
        task: String,

        /// Working directory for the worker
        #[arg(short, long, default_value = ".")]
        dir: String,

        /// Label (unused, reserved)
        #[arg(short, long)]
        name: Option<String>,

        /// Model to use (e.g. "fireworks/kimi-k2.5", "anthropic/claude-sonnet-4-6")
        #[arg(short, long)]
        model: Option<String>,
    },

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
        }) => {
            let pane = tmux::spawn(&task, &dir, name.as_deref(), model.as_deref())?;
            let out = serde_json::json!({ "pane": pane });
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
        Some(Command::LoopStatus { pane }) => {
            match pane {
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
                    // Check all known panes
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
            }
        }
        Some(Command::LoopClear { pane }) => {
            loop_guard::clear_pane(&pane)?;
            let out = serde_json::json!({ "pane": pane, "cleared": true });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }

    Ok(())
}
