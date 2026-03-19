# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-03-19

### Added

- **Multi-harness support** — Added support for multiple AI coding agents:
  - OpenCode (default)
  - Claude Code
  - Codex CLI
  - Easily switch between harnesses with F2 key or `superharness harness-set`
  - Per-harness model configuration support

- **tmux attach support** — Launch superharness inside existing tmux sessions without conflicts

- **Automatic pane management** — Scanner daemon now automatically manages pane lifecycle:
  - Surfaces active workers, backgrounds idle ones
  - Keeps 2-3 worker panes visible for optimal workspace organization

- **Stale worker detection daemon** — Automatic detection and cleanup of crashed or stuck workers via heartbeat thread

- **extract_busy_state() improvements** — Complete rewrite of busy detection logic:
  - Single-snapshot pattern matching (no more 500ms sleep diff)
  - Support for opencode (`esc interrupt`, `esc again to interrupt`, braille spinners)
  - Support for Codex CLI (`esc to interrupt`, `Working` indicators)
  - Support for Claude Code (braille spinner detection)
  - Comprehensive test suite (66+ tests)

- **Harness switch restart** — Orchestrator pane automatically restarts when switching harness via F2

- **Universal input detection** — TUI-aware `get_prompt_text()` for detecting user input across all harness types

- **Worker identity header** — Prevents workers from accidentally acting as superharness orchestrator

- **F4 status bar** — Compact status display showing current harness, connected workers, and pending tasks

### Changed

- **AGENTS.md improvements**:
  - Streamlined template from 478 to 182 lines (~70% token reduction)
  - Added model selection guide
  - Documented event-driven architecture
  - Fixed infinite-append bug with interactive merge support

- **Heartbeat system overhaul**:
  - Replaced tmux daemon window with background thread
  - Self-healing daemon with automatic recovery
  - Simplified heartbeat message to `[HEARTBEAT]`
  - Atomic state writes for TOCTOU safety
  - 50+ comprehensive unit tests

- **Output processing refinements**:
  - Narrowed box character stripping to preserve indentation
  - Removed strip_right_panel to preserve content
  - Better handling of multi-line input detection

### Fixed

- Permission bypass flags removed from Claude and Codex harnesses for security
- Shell-only guard properly removed from `main_pane_has_input()`
- Cursor position checks improved for multi-line input scenarios
- Box-drawing and block element character handling in `line_has_content`
- Ghost text elimination (removed `eprintln!` calls from heartbeat)
- Heartbeat daemon window index collision and dangling window cleanup
- Homebrew and AUR asset URL corrections
- Install script target corrections (musl → gnu)

## [0.3.0] - 2026-03-05

### Added

- Initial heartbeat system with snooze and countdown
- TUI-aware busy detection with spinner recognition
- Git worktree isolation per worker
- Task dependency system with `--depends-on`
- Loop detection for stuck agents
- Health monitoring with auto-recovery
- Auto-watch mode for full supervision
- Away mode for queuing decisions
- Checkpointing for long-running sessions
- Pane management commands (hide, surface, compact, resize, layout)
- Pulse digest status summaries

## [0.2.1] - 2026-02-28

### Fixed

- Minor bug fixes and stability improvements

## [0.2.0] - 2026-02-20

### Added

- Initial public release
- Spawn workers with `superharness spawn`
- Read and send commands for worker interaction
- Task dependency management
- Basic health monitoring

[0.4.0]: https://github.com/backmeupplz/superharness/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/backmeupplz/superharness/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/backmeupplz/superharness/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/backmeupplz/superharness/releases/tag/v0.2.0
