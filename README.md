# ![superharness](docs/favicon.svg) superharness

[![Crates.io](https://img.shields.io/crates/v/superharness)](https://crates.io/crates/superharness)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/backmeupplz/superharness)](https://github.com/backmeupplz/superharness)

**Multi-agent orchestration for Claude Code, OpenCode, and Codex CLI.**

superharness sits on top of your AI coding agent and gives it the ability to spawn parallel worker agents, manage them in tmux, handle permissions, detect stalls, and clean up — all autonomously. You just run `superharness` in your project instead of your agent directly.

## Quick start

```bash
cargo install superharness
cd your-project
superharness
```

That's it. You get the normal agent interface you already know — but now the AI can spawn parallel workers, coordinate them across isolated git worktrees, auto-approve safe operations, detect stuck workers, and clean up — all without you lifting a finger.

## How it works

1. **Install** — `cargo install superharness`. Requires tmux and an AI coding agent (Claude Code, OpenCode, or Codex CLI).
2. **Run** — `superharness` in your project directory.
3. **Work** — The AI orchestrator spawns parallel worker agents when the task needs them. Each worker runs in its own git worktree. The orchestrator reviews permission prompts, auto-approves safe operations, detects stalled workers, merges results, and cleans up — without you doing anything.
4. **Optional: Away Mode** — Step away and superharness keeps working autonomously, queuing uncertain decisions for your return.

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

Requires: [tmux](https://github.com/tmux/tmux) · an AI coding agent ([Claude Code](https://claude.ai/code), [OpenCode](https://opencode.ai), or [Codex CLI](https://openrouter.ai))

---

## Going somewhere?

superharness has an away mode for when you step out. The AI keeps workers running and handles safe operations, but queues any real decision — architecture choices, destructive operations, anything it isn't sure about — for your return. Full debrief when you're back.

---

## Features

**Multi-Harness Support**
- Works seamlessly with Claude Code, OpenCode, and Codex CLI
- Unified orchestration across different AI agents
- Automatic busy/idle detection per harness

**Tmux Management**
- Spawns workers in isolated panes and git worktrees
- Auto-surfaces workers needing attention
- Hides idle workers, keeps main window clean
- Permission prompts are auto-approved for safe operations (edits, git, builds, tests)

**Task Decomposition & Parallelization**
- Breaks work into independent task units
- Spawns multiple workers simultaneously
- Tracks progress across all tasks
- Respawns crashed workers with context

**Away Mode**
- Run `superharness` and step away
- AI keeps workers running autonomously
- Safe operations auto-approved, uncertain decisions queued
- Get a full debrief when you return

**Heartbeat System**
- Detects stuck/idle workers
- Wakes the orchestrator immediately when work is done
- No polling, no artificial sleep — event-driven throughout

**Intelligent Detection**
- Detects busy/idle state per harness (spinners, permission prompts, keybind hints)
- Automatically re-runs blocked tasks when dependencies finish
- Cleans up worktrees on exit

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
