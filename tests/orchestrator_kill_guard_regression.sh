#!/usr/bin/env bash
#
# Regression test: `superharness kill` must REFUSE to kill the orchestrator pane.
#
# Real-world crash: with a worker in `--mode plan` (hidden, silent), something
# issued `kill --pane %0` — `%0` is the orchestrator. The event log showed
# repeated `worker_killed pane:%0`. Killing the orchestrator ends the tmux
# session and the whole superharness process exits ("crashed"). The worker
# self-kill was proven safe (it uses the worker's own $TMUX_PANE), so the bad
# kill comes from the orchestrator itself targeting %0. The fix is a safety net:
# `tmux::kill` refuses any pane that resolves to the orchestrator (by the
# `@sh_orchestrator` tag or by matching the resolved orchestrator pane id).
#
# This runs under base-index 1 / pane-base-index 1 (the user's config).
#
# ISOLATION: own tmux socket + TMUX_TMPDIR; never touches the user's server.
#
# Run:  bash tests/orchestrator_kill_guard_regression.sh

set -u
BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/superharness"
[ -x "$BIN" ] || { echo "FAIL: release binary not found at $BIN (run: cargo build --release)" >&2; exit 1; }

unset TMUX TMUX_PANE
export TMUX_TMPDIR; TMUX_TMPDIR="$(mktemp -d /tmp/sh-guard.XXXXXX)"
SOCK="sh-guard-$$"
tx(){ tmux -L "$SOCK" "$@"; }
cleanup(){ tmux -L "$SOCK" kill-server 2>/dev/null; rm -rf "$TMUX_TMPDIR"; }
trap cleanup EXIT
fail(){ echo "FAIL: $1" >&2; exit 1; }
alive(){ tx list-panes -a -F '#{pane_id}' 2>/dev/null | grep -qx "$1"; }

tx -f /dev/null start-server
tx set-option -g base-index 1
tx set-option -g pane-base-index 1
tx new-session -d -s superharness -x 200 -y 50
orch="$(tx display-message -p -t superharness '#{pane_id}')"
tx set-option -p -t "$orch" @sh_orchestrator 1
tx set-environment -t superharness SUPERHARNESS_ORCH_PANE "$orch"
sockpath="$(tx display-message -p '#{socket_path}')"; sid="$(tx display-message -p -t superharness '#{session_id}')"
export TMUX="$sockpath,0,${sid#\$}"
echo "orchestrator=$orch (base-index 1)"

# A real worker pane (tagged), as spawn would create.
w="$(tx split-window -d -t superharness -P -F '#{pane_id}' -c /tmp bash -lc 'sleep 300')"
tx set-option -p -t "$w" @sh_worker 1
echo "worker=$w"

# 1. Killing the orchestrator MUST be refused, and the orchestrator MUST survive.
out="$($BIN kill --pane "$orch" 2>&1)"; rc=$?
echo "kill orchestrator: rc=$rc msg=$(echo "$out" | tr '\n' ' ' | head -c 120)"
[ $rc -ne 0 ] || fail "kill --pane <orchestrator> returned success (should be refused)"
echo "$out" | grep -qi "refus" || fail "refusal message not shown for orchestrator kill"
alive "$orch" || fail "ORCHESTRATOR $orch was killed despite the guard — REGRESSION"
echo "PASS-1: orchestrator kill refused; orchestrator still alive"

# 2. A real worker MUST still be killable.
$BIN kill --pane "$w" >/dev/null 2>&1 || fail "killing a real worker failed"
sleep 1
alive "$w" && fail "worker $w survived an explicit kill"
alive "$orch" || fail "orchestrator died while killing a worker"
echo "PASS-2: real worker killed; orchestrator unaffected"

echo "PASS: kill refuses the orchestrator, still kills workers"
