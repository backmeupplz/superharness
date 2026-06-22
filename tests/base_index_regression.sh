#!/usr/bin/env bash
#
# Regression test for the "can't find window: 0" / failed-surface bug.
#
# superharness used to hardcode the orchestrator window as `superharness:0`
# (window *index* 0) throughout the layout/surface/compact code. Users with
# `base-index 1` in their tmux config get the orchestrator window at index 1,
# so every window-0-targeted operation failed with `can't find window: 0` and
# surfacing background workers was impossible.
#
# The fix:
#   1. Target the orchestrator window by its stable window *ID* (#{window_id}),
#      derived dynamically from the orchestrator pane — index-independent.
#   2. Tag worker panes with the `@sh_worker` pane option at spawn and only ever
#      manage tagged panes, so user-created tmux windows are left alone.
#
# This test runs entirely on a private tmux socket (`-L shregress`) under
# `base-index 1`, replicating the exact command sequences the Rust code emits.
# It never touches the user's real tmux server/sessions.
#
# Run:  bash tests/base_index_regression.sh
# Exits 0 on success, non-zero (with a message) on regression.

set -u
T="tmux -L shregress"
fail() { echo "FAIL: $1" >&2; $T kill-server 2>/dev/null; exit 1; }

$T kill-server 2>/dev/null
$T -f /dev/null start-server
$T set-option -g base-index 1      # reproduce the user's environment
$T set-option -g pane-base-index 1

$T new-session -d -s superharness -x 220 -y 50

# The orchestrator window must NOT be at index 0 under base-index 1 — this is
# precisely the condition that broke the old hardcoded `superharness:0`.
idx=$($T list-windows -t superharness -F '#{window_index}')
[ "$idx" = "0" ] && fail "expected base-index 1 to place the window off index 0 (got $idx)"

orch=$($T display-message -p -t superharness '#{pane_id}')
orchwin=$($T display-message -p -t "$orch" '#{window_id}')   # the fix: dynamic window id

# Old hardcoded target must fail; new dynamic target must work.
$T list-panes -t superharness:0 >/dev/null 2>&1 && fail "superharness:0 unexpectedly resolved"
$T list-panes -t "$orchwin" >/dev/null 2>&1 || fail "dynamic window id $orchwin did not resolve"

# Spawn + tag a worker, hide it, and confirm the tag survives break-pane.
worker=$($T split-window -t superharness -d -P -F '#{pane_id}')
$T set-option -p -t "$worker" @sh_worker 1
$T break-pane -s "$worker" -d -n wk
tag=$($T show-options -p -t "$worker" @sh_worker)
[ "$tag" = "@sh_worker 1" ] || fail "@sh_worker tag did not survive break-pane (got '$tag')"

# Surface: old target fails, new target succeeds.
$T join-pane -s "$worker" -t superharness:0 -h -d 2>/dev/null && fail "surface to superharness:0 unexpectedly worked"
$T join-pane -s "$worker" -t "$orchwin" -h -d 2>/dev/null || fail "surface to dynamic window id failed"

# A user-created window/pane must be excluded from the worker-filtered list.
$T new-window -t superharness -d
workers=$($T list-panes -t superharness -a -F '#{pane_id} #{?@sh_worker,1,0}' | awk '$2=="1"{print $1}')
[ "$workers" = "$worker" ] || fail "worker filter wrong; expected only $worker, got: $workers"

$T kill-server 2>/dev/null
echo "PASS: base-index 1 surface/worker-isolation regression test"
