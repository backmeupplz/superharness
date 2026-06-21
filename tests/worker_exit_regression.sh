#!/usr/bin/env bash
#
# Regression test for the "worker exit kills the orchestrator" bug.
#
# When a worker finished, its wrapped shell command resolved the pane to kill
# with an UNTARGETED `tmux display-message -p '#{pane_id}'`. With no `-t`, tmux
# returns the session's *active* pane. Workers are spawned with `-d` so focus
# stays on the orchestrator — meaning the active pane is the orchestrator. So a
# finishing worker ran `superharness kill --pane %<orchestrator>` and killed the
# orchestrator (and thus the whole superharness process/session) instead of
# itself.
#
# The fix: resolve the worker's OWN pane from the per-pane `$TMUX_PANE` env var
# (set by tmux per pane), guarded so an empty value can never fall through to the
# active pane.
#
# ISOLATION (so this test can NEVER touch the user's real tmux server or the
# running superharness session — the exact thing the bug crashes):
#   1. unset TMUX / TMUX_PANE          -> detach from any session we run inside.
#   2. export a fresh TMUX_TMPDIR      -> tmux's default socket lives in a temp dir.
#   3. pin every harness-level tmux to -L "$SOCK" (a unique named socket)
#      via the tx() wrapper            -> commands provably hit only our server.
# All three are independent; any one alone is sufficient. `kill-server` only
# ever targets "$SOCK", so it cannot reach the user's default server.
#
# Run:  bash tests/worker_exit_regression.sh
# Exits 0 on success, non-zero (with a message) on regression.

set -u

BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/superharness"
[ -x "$BIN" ] || { echo "FAIL: release binary not found at $BIN (run: cargo build --release)" >&2; exit 1; }

# Detach from any tmux session we may be running inside.
unset TMUX TMUX_PANE
# Fully isolated tmux server: its own socket directory, never the user's.
export TMUX_TMPDIR
TMUX_TMPDIR="$(mktemp -d /tmp/sh-wtest.XXXXXX)"
# Unique named socket — every harness-level tmux call goes through tx() so it
# can only ever reach THIS server, never the user's default one.
SOCK="sh-wtest-$$"

tx() { tmux -L "$SOCK" "$@"; }

cleanup() { tmux -L "$SOCK" kill-server 2>/dev/null; rm -rf "$TMUX_TMPDIR"; }
trap cleanup EXIT
fail() { echo "FAIL: $1" >&2; exit 1; }

pane_alive() { tx list-panes -a -F '#{pane_id}' 2>/dev/null | grep -qx "$1"; }

# ---------------------------------------------------------------------------
# Part 1 — reproduce the OLD bug to prove the harness actually exercises it.
# ---------------------------------------------------------------------------
tx kill-server 2>/dev/null
tx -f /dev/null start-server
tx new-session -d -s test -x 200 -y 50
orch="$(tx display-message -p -t test '#{pane_id}')"
[ -n "$orch" ] || fail "could not determine orchestrator pane id"
echo "orchestrator pane = $orch"

# OLD worker-exit form: untargeted display-message -> active pane (orchestrator).
# The inner `tmux` inherits this pane's $TMUX, so it resolves against THIS server.
old_cmd="echo old-worker-ran; $BIN kill --pane \$(tmux display-message -p '#{pane_id}')"
worker_old="$(tx split-window -d -t test -P -F '#{pane_id}' bash -lc "$old_cmd")"
echo "old-style worker pane = $worker_old (spawned with -d; $orch stays active)"

sleep 2  # let the worker's bash run echo + kill

if pane_alive "$orch"; then
  echo "  unexpected: old form did not kill orchestrator on this tmux — bug not reproduced"
else
  echo "  reproduced: OLD form killed the orchestrator ($orch gone) — the bug is real"
fi
tx kill-server 2>/dev/null

# ---------------------------------------------------------------------------
# Part 2 — the FIX: real binary, worker kills its OWN pane via $TMUX_PANE.
# ---------------------------------------------------------------------------
tx -f /dev/null start-server
tx new-session -d -s test -x 200 -y 50
orch="$(tx display-message -p -t test '#{pane_id}')"
[ -n "$orch" ] || fail "could not determine orchestrator pane id (part 2)"
echo "orchestrator pane = $orch"

# NEW worker-exit form (matches build_worker_exit_cmd in src/tmux/panes.rs):
new_cmd="echo new-worker-ran; [ -n \"\$TMUX_PANE\" ] && $BIN kill --pane \"\$TMUX_PANE\""
worker_new="$(tx split-window -d -t test -P -F '#{pane_id}' bash -lc "$new_cmd")"
echo "new-style worker pane = $worker_new (spawned with -d; $orch stays active)"

sleep 2  # let the worker's bash run echo + self-kill

pane_alive "$orch"        || fail "orchestrator $orch was killed by a finishing worker — REGRESSION"
pane_alive "$worker_new"  && fail "worker $worker_new did not kill itself on exit"

echo "PASS: finishing worker killed its own pane ($worker_new) and left the orchestrator ($orch) alive"
