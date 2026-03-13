# ![superharness](docs/favicon.svg) superharness

[![Crates.io](https://img.shields.io/crates/v/superharness)](https://crates.io/crates/superharness)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/backmeupplz/superharness)](https://github.com/backmeupplz/superharness)

**Your AI coding team. Working while you sleep.**

SuperHarness is an autonomous AI coding team manager. You describe what you want to build, it plans the work, spawns parallel AI agents, supervises them through the night, and gives you a full debrief when you return. Replace `opencode` with `superharness` — that's the whole migration.

## Quick start

```bash
cargo install superharness
cd your-project
superharness plan "build a REST API with JWT auth"
superharness away
# go to sleep
superharness present
```

## How it works

- **Plan** — Describe your goal in one sentence. SuperHarness creates a staged roadmap and assigns tasks to parallel agents automatically.
- **Execute** — A team of AI agents works in isolated git worktrees while you're away. Smart supervision handles stalls, loops, and safe permission prompts without waking you.
- **Debrief** — `superharness present` shows everything: what was built, what's in progress, and any decisions waiting for you.

## Install

```bash
cargo install superharness
```

Requires: [tmux](https://github.com/tmux/tmux) · brew and AUR packages coming soon

---

## Advanced

Full command reference for power users and orchestrator scripting.

| Command | Description |
|---|---|
| `superharness` | Initialize and open tmux session |
| `plan` | Describe a goal; SuperHarness creates a roadmap and spawns agents |
| `away` | Enter away mode; queue critical decisions for review on return |
| `present` | Return to present mode and see the full debrief |
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
