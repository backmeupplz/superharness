# ![superharness](docs/favicon.svg) superharness

[![Crates.io](https://img.shields.io/crates/v/superharness)](https://crates.io/crates/superharness)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/backmeupplz/superharness)](https://github.com/backmeupplz/superharness)

**Turn any AI coding agent into a multi-agent orchestrator — just run `superharness` in your project.**

superharness wraps [opencode](https://opencode.ai) and tmux into a self-managing multi-agent system. One agent becomes the orchestrator; it spawns, monitors, and coordinates workers — each isolated in its own git worktree, each running a different task in parallel.

## Features

- **Parallel workers** — spawn agents in isolated git worktrees so they never conflict
- **Plan + Build modes** — `plan` for read-only analysis, `build` for full execution
- **Task dependencies** — DAG-style `--depends-on` queuing; pending tasks spawn automatically when blockers finish
- **Continuous monitoring** — `monitor` polls panes for stalls and attempts auto-recovery
- **Away mode** — queue critical decisions when you're not watching; review them when you return
- **Loop detection** — tracks repeated inputs to a pane and surfaces oscillation patterns before they waste time
- **Checkpoints** — snapshot any pane's output and resume it later with a fresh worker
- **Per-pane memory** — store structured key-value facts across a session for any pane

## Demo

```
$ superharness
# Opens tmux session. You are the orchestrator in pane %0.

$ superharness spawn --task "Refactor auth module" --dir /tmp/w1 --model anthropic/claude-sonnet-4-6
# => { "pane": "%1" }  — worker running in isolated git worktree

$ superharness workers
# %1  [build]  ACTIVE   /tmp/w1   "Refactor auth module"
# %2  [plan]   IDLE     /tmp/w2   "Audit test coverage"

$ superharness watch
# Auto-managing all panes: approving safe prompts, nudging stalls, cleaning finished workers...
```

## Install

```bash
cargo install superharness
```

Requires: [tmux](https://github.com/tmux/tmux) and [opencode](https://opencode.ai)

> **Coming soon:**
> ```bash
> brew install backmeupplz/tap/superharness   # Homebrew
> yay -S superharness-bin                      # AUR
> ```

## Usage

```bash
# 1. Launch the orchestrator in your project
cd your-project
superharness

# This opens a tmux session with you (the AI) as orchestrator.
# From here, run superharness subcommands to manage workers.

# 2. Spawn a worker in an isolated worktree
git worktree add /tmp/w1 HEAD
superharness spawn --task "Refactor auth module" --dir /tmp/w1 --model anthropic/claude-sonnet-4-6

# 3. Spawn with dependencies (DAG execution)
superharness spawn --task "Run integration tests" --dir /tmp/w2 \
  --depends-on "%23" --model anthropic/claude-sonnet-4-6

# 4. Check pending dependency-gated tasks
superharness tasks

# 5. Promote ready tasks once blockers finish
superharness run-pending

# 6. Monitor all panes — auto-recovers stalls
superharness monitor --interval 60 --stall-threshold 3

# 7. One-shot health snapshot
superharness healthcheck

# 8. Read worker output
superharness read --pane %23 --lines 100

# 9. Send input to a worker
superharness send --pane %23 --text "y"
```

## How it works

- `superharness` (no subcommand) writes an `AGENTS.md` config into your project and opens a tmux session. The AI agent in the main pane reads `AGENTS.md` and acts as orchestrator.
- Workers are spawned as new tmux panes running opencode, each with a task prompt injected at startup.
- State (mode, decisions, checkpoints, loop history, memory) is persisted in `~/.local/share/superharness/` and survives restarts.
- `monitor` runs a polling loop that hashes pane output; unchanged output past the stall threshold triggers progressive recovery attempts (Enter → `continue` → human escalation).

## Commands

| Command | Description |
|---|---|
| `superharness` | Initialize and open tmux session |
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
| `watch` | Auto follow-up loop: cleanup done panes, approve safe prompts, nudge stalled panes |
| `healthcheck` | One-shot structured health snapshot |
| `away / present` | Toggle human-watching mode |
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
