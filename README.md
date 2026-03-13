# ![superharness](docs/favicon.svg) superharness

[![Crates.io](https://img.shields.io/crates/v/superharness)](https://crates.io/crates/superharness)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/backmeupplz/superharness)](https://github.com/backmeupplz/superharness)

**Give opencode a team.**

superharness sits on top of [opencode](https://opencode.ai) and gives it the ability to spawn parallel worker agents, manage them, and clean up — all on its own. You just run `superharness` in your project instead of `opencode`.

## Quick start

```bash
cargo install superharness
cd your-project
superharness
```

That's it. You get a normal opencode session — the same interface you already know — but now the AI can spawn as many parallel workers as the task needs, coordinate them across isolated git worktrees, handle permission prompts, and clean up when they're done.

## How it works

1. **Install** — `cargo install superharness`. Requires tmux and opencode.
2. **Run** — `superharness` in your project directory.
3. **Work** — opencode spawns worker agents in parallel when the task warrants it. Each worker runs in its own git worktree. The orchestrating AI reviews permission prompts, detects stalled workers, merges results, and cleans up — without you doing anything.

## Install

### Quick install

```bash
curl -fsSL https://superharness.dev/install.sh | sh
```

### Ubuntu/Debian (PPA)

```bash
sudo add-apt-repository ppa:borodutch/superharness
sudo apt update
sudo apt install superharness
```

### Arch Linux (AUR)

```bash
yay -S superharness
```

### macOS/Linux (Homebrew)

```bash
brew install backmeupplz/superharness/superharness
```

### Cargo

```bash
cargo install superharness
```

Requires: [tmux](https://github.com/tmux/tmux) · an AI coding agent ([opencode](https://opencode.ai), claude, or codex)

---

## Going somewhere?

superharness has an away mode for when you step out. The AI keeps workers running and handles safe operations, but queues any real decision — architecture choices, destructive operations, anything it isn't sure about — for your return. Full debrief when you're back.

---

## Advanced

Full command reference for power users and scripting. You don't need any of these day-to-day — just `superharness`.

| Command | Description |
|---|---|
| `superharness` | Start opencode in orchestrator mode |
| `spawn` | Create a new worker pane |
| `list` | List all active panes (JSON) |
| `workers` | List workers in human-readable format (F4 popup) |
| `status-human` | Human-readable status + worker health (F3 popup) |
| `read --pane %ID` | Read recent output from a pane |
| `send --pane %ID --text "..."` | Send input to a pane |
| `ask --pane %ID` | Detect if a worker is asking a question |
| `kill --pane %ID` | Kill a pane |
| `hide / show` | Move pane to background tab or surface it |
| `resize / layout` | Adjust pane geometry |
| `compact` | Move small/excess panes to background tabs |
| `surface --pane %ID` | Bring a background pane back to main window |
| `git-check --dir /path` | Verify repo is clean before creating a worktree |
| `respawn --pane %ID` | Kill crashed worker and respawn with crash context |
| `tasks` | List pending (dependency-gated) tasks |
| `run-pending` | Spawn tasks whose dependencies have finished |
| `monitor` | Continuous stall detection and auto-recovery |
| `watch` | Auto-manage all panes: cleanup, approve safe prompts, nudge stalls |
| `healthcheck` | One-shot structured health snapshot |
| `status` | Show current mode and queued decisions |
| `queue-decision` | Queue a decision for human review |
| `clear-decisions` | Clear resolved decisions |
| `loop-status` | Show loop detection state for pane(s) |
| `loop-clear` | Reset loop history after intervention |
| `checkpoint` | Snapshot a pane's output |
| `checkpoints` | List saved checkpoints |
| `resume` | Spawn a new worker from a checkpoint |
| `memory` | Store or list key-value facts for a pane |

## License

MIT — see [LICENSE](LICENSE)

---

Website: [superharness.dev](https://superharness.dev) · GitHub: [backmeupplz/superharness](https://github.com/backmeupplz/superharness)
