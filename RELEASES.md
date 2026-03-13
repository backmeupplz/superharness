# Release Notes

## v0.2.0

Initial public release.

### Features

- **Spawn workers** — launch AI coding agent panes in tmux with `superharness spawn`
- **Read & send** — poll worker output and relay input with `superharness read` / `superharness send`
- **Git worktree isolation** — each worker gets its own worktree via `git-check` + `git worktree add`
- **Task dependencies** — queue tasks that wait for a prerequisite pane to finish (`--depends-on`)
- **Loop detection** — automatic detection of agents stuck in repetitive cycles (`loop-status`, `loop-clear`)
- **Health monitoring** — `monitor` subcommand watches for stalled workers and attempts recovery
- **Auto-watch** — `watch` subcommand fully supervises all panes, approves safe prompts, and cleans up
- **Away mode** — queue decisions while the human is absent; debrief on return
- **Checkpointing** — periodic state snapshots for long-running sessions
- **Pending tasks** — `tasks` / `run-pending` for dependency-aware workflow orchestration
- **Pane management** — `hide`, `surface`, `compact`, `resize`, `layout` presets
- **Pulse digest** — `pulse` sends a status summary to the orchestrator pane

### Requirements

- [tmux](https://github.com/tmux/tmux) ≥ 3.0
- [opencode](https://opencode.ai)
