#!/usr/bin/env bash
#
# Regression test for the "worker hangs in bash and never runs claude" bug.
#
# Workers were spawned with the harness in headless one-shot mode
# (`claude -p <task>` / `codex exec <task>`). Print/exec mode launches a
# *headless* process with no interactive TUI, so the worker pane's
# `pane_current_command` stayed `bash` for the worker's whole life — the agent
# was invisible and unmonitorable, and the pane appeared to hang in bash forever
# without ever visibly running claude.
#
# The fix: workers launch the harness INTERACTIVELY (`claude <task>` /
# `codex <task>`), exactly like the orchestrator and like opencode workers. This
# test drives the REAL binary's `spawn` and asserts the worker pane's actual
# start command is the interactive form, never the headless one-shot form.
#
# ISOLATION (so this test can NEVER touch the user's real tmux server or the
# running superharness session):
#   1. unset TMUX / TMUX_PANE          -> detach from any session we run inside.
#   2. export a fresh TMUX_TMPDIR      -> tmux's default socket lives in a temp dir.
#   3. pin every harness-level tmux to -L "$SOCK" (a unique named socket).
# We also point the binary's own (un-pinned) `tmux` calls at THIS server by
# exporting $TMUX to its socket, so spawn provably acts only on our server.
#
# Run:  bash tests/worker_interactive_regression.sh
# Exits 0 on success, non-zero (with a message) on regression.

set -u

BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/superharness"
[ -x "$BIN" ] || { echo "FAIL: release binary not found at $BIN (run: cargo build --release)" >&2; exit 1; }

unset TMUX TMUX_PANE
export TMUX_TMPDIR
TMUX_TMPDIR="$(mktemp -d /tmp/sh-itest.XXXXXX)"
SOCK="sh-itest-$$"

tx() { tmux -L "$SOCK" "$@"; }
cleanup() { tmux -L "$SOCK" kill-server 2>/dev/null; rm -rf "$TMUX_TMPDIR"; }
trap cleanup EXIT
fail() { echo "FAIL: $1" >&2; exit 1; }

# Spawn a worker for the given harness and return its pane start command.
worker_start_cmd() {
  local harness="$1"
  tx kill-server 2>/dev/null
  tx -f /dev/null start-server
  tx new-session -d -s superharness -x 200 -y 50
  local orch
  orch="$(tx display-message -p -t superharness '#{pane_id}')"
  [ -n "$orch" ] || fail "could not determine orchestrator pane id ($harness)"
  # Mimic what session::init() does so spawn targeting resolves correctly.
  tx set-option -p -t "$orch" @sh_orchestrator 1
  tx set-environment -t superharness SUPERHARNESS_ORCH_PANE "$orch"

  # Point the binary's own tmux calls at THIS isolated server.
  local sockpath sid
  sockpath="$(tx display-message -p '#{socket_path}')"
  sid="$(tx display-message -p -t superharness '#{session_id}')"
  export TMUX="$sockpath,0,${sid#\$}"

  # --no-hide keeps the worker in the main window; --harness pins the harness so
  # the test does not depend on what is installed/configured locally.
  "$BIN" spawn --task "do the task" --dir /tmp --name w --harness "$harness" --no-hide \
    >/dev/null 2>&1 || fail "spawn failed for harness=$harness"

  local wpane
  wpane="$(tx list-panes -a -F '#{pane_id} #{@sh_worker}' | awk '$2==1{print $1; exit}')"
  [ -n "$wpane" ] || fail "no @sh_worker-tagged pane after spawn ($harness)"
  tx list-panes -a -F '#{pane_id}|#{pane_start_command}' | awk -F'|' -v w="$wpane" '$1==w{print $2}'
}

# ── claude ──────────────────────────────────────────────────────────────────
cmd="$(worker_start_cmd claude)"
echo "claude worker start cmd: $cmd"
case "$cmd" in
  *"claude -p"*) fail "claude worker runs headless '-p' print mode (the bug): $cmd" ;;
esac
case "$cmd" in
  *"claude '"*|*"claude --model"*) : ;;
  *) fail "claude worker does not invoke interactive claude: $cmd" ;;
esac

# ── codex ─────────────────────────────────────────────────────────────────────
cmd="$(worker_start_cmd codex)"
echo "codex worker start cmd: $cmd"
case "$cmd" in
  *"codex exec"*) fail "codex worker runs headless 'exec' mode (the bug): $cmd" ;;
esac

echo "PASS: workers launch their harness interactively (no headless -p / exec)"
