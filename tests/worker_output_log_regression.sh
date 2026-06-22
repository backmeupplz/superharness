#!/usr/bin/env bash
#
# Regression test: a worker's output must survive its pane being killed.
#
# Workers self-destruct on completion (the spawn wrapper appends
# `; superharness kill --pane "$TMUX_PANE"`), so once a worker finishes its pane
# and scrollback are gone and the output is no longer reviewable in-pane. The
# fix captures the pane's full scrollback to
# `{project}/.superharness/worker-logs/<title>-<paneid>.log` right before the
# pane is destroyed. Every kill path (worker self-kill AND orchestrator-/retry-
# initiated kills) goes through `tmux::kill`, so the capture covers them all.
#
# This test drives the REAL `superharness kill` command against a pane holding a
# known marker and asserts the marker lands in a log file and the pane is gone.
#
# ISOLATION (so the test can NEVER touch the user's tmux server, real project,
# or real ~/.local/share state):
#   1. unset TMUX / TMUX_PANE          -> detach from any session we run inside.
#   2. fresh TMUX_TMPDIR + named -L socket via tx().
#   3. HOME pointed at a throwaway dir -> active_project.txt is resolved there,
#      and points at a throwaway project dir, so logs land in the temp project.
#   4. the binary's own tmux calls are pinned to THIS server via $TMUX.
#
# Run:  bash tests/worker_output_log_regression.sh
# Exits 0 on success, non-zero (with a message) on regression.

set -u

BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/superharness"
[ -x "$BIN" ] || { echo "FAIL: release binary not found at $BIN (run: cargo build --release)" >&2; exit 1; }

unset TMUX TMUX_PANE
export TMUX_TMPDIR
TMUX_TMPDIR="$(mktemp -d /tmp/sh-logtest.XXXXXX)"
SOCK="sh-logtest-$$"

# Throwaway HOME + project so we never read or write the user's real state.
FAKE_HOME="$(mktemp -d /tmp/sh-loghome.XXXXXX)"
PROJECT="$(mktemp -d /tmp/sh-logproj.XXXXXX)"
mkdir -p "$FAKE_HOME/.local/share/superharness"
printf '%s' "$PROJECT" > "$FAKE_HOME/.local/share/superharness/active_project.txt"
export HOME="$FAKE_HOME"

tx() { tmux -L "$SOCK" "$@"; }
cleanup() {
  tmux -L "$SOCK" kill-server 2>/dev/null
  rm -rf "$TMUX_TMPDIR" "$FAKE_HOME" "$PROJECT"
}
trap cleanup EXIT
fail() { echo "FAIL: $1" >&2; exit 1; }
pane_alive() { tx list-panes -a -F '#{pane_id}' 2>/dev/null | grep -qx "$1"; }

MARKER="WORKER_OUTPUT_MARKER_$$_abc123"

tx -f /dev/null start-server
tx new-session -d -s superharness -x 200 -y 50

# A "worker" pane that emits a known marker then lingers (so we control when it
# dies, via the real kill command — exactly the self-kill code path).
worker="$(tx split-window -d -t superharness -P -F '#{pane_id}' \
  bash -lc "echo $MARKER; echo second line of output; sleep 60")"
[ -n "$worker" ] || fail "could not spawn worker pane"
# Stable superharness label (set by spawn). Also set a DIFFERENT live pane_title
# to mimic claude overwriting it — the log filename must come from @sh_label,
# not the harness-controlled pane_title.
tx set-option -p -t "$worker" @sh_label "[build] log test"
tx select-pane -t "$worker" -T "Some Claude Self Assigned Title"
sleep 1  # let the echoes hit the pane scrollback

# Point the binary's tmux calls at THIS server.
sockpath="$(tx display-message -p '#{socket_path}')"
sid="$(tx display-message -p -t superharness '#{session_id}')"
export TMUX="$sockpath,0,${sid#\$}"

# Real kill — this is what the worker self-kill wrapper runs on completion.
"$BIN" kill --pane "$worker" >/dev/null 2>&1 || fail "superharness kill failed"

sleep 1
pane_alive "$worker" && fail "worker pane $worker still alive after kill"

LOG_DIR="$PROJECT/.superharness/worker-logs"
[ -d "$LOG_DIR" ] || fail "worker-logs dir not created at $LOG_DIR"

logfile="$(ls "$LOG_DIR"/*.log 2>/dev/null | head -1)"
[ -n "$logfile" ] || fail "no worker log file written in $LOG_DIR"
echo "worker log: $logfile"

grep -q "$MARKER" "$logfile" || fail "worker output marker not found in log $logfile"

# Filename must derive from the stable @sh_label, NOT the harness-overwritten
# pane_title ("Some Claude Self Assigned Title").
case "$(basename "$logfile")" in
  build_log_test-*.log) : ;;
  Some_Claude*) fail "log filename used the volatile pane_title, not @sh_label: $(basename "$logfile")" ;;
  *) fail "unexpected log filename: $(basename "$logfile") (expected build_log_test-<id>.log)" ;;
esac

echo "PASS: worker output captured to $logfile before the pane was killed"
