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
}

#[derive(clap::Subcommand)]
enum Command {
    /// Spawn a new opencode worker in its own tmux window
    Spawn {
        /// Task/prompt to give the worker
        #[arg(short, long)]
        task: String,

        /// Working directory for the worker
        #[arg(short, long, default_value = ".")]
        dir: String,

        /// Window name (shown in tmux tab bar)
        #[arg(short, long)]
        name: Option<String>,
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            setup::write_config(&cli.dir)?;
            tmux::init(&cli.dir)?;
        }
        Some(Command::Spawn { task, dir, name }) => {
            let pane = tmux::spawn(&task, &dir, name.as_deref())?;
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
    }

    Ok(())
}
