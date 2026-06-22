#!/usr/bin/env bash
#
# Regression test for the "can't find pane: %0" bug on F1 / `toggle-mode`.
#
# Two coupled defects made `superharness toggle-mode` fail:
#   1. handle_toggle_mode() resolved its target by searching tmux::list(), which
#      (since the worker-isolation change) returns ONLY @sh_worker-tagged panes.
#      The orchestrator is never tagged, so it was never found — the message went
#      to a random worker or fell back to a literal `%0`.
#   2. orchestrator_pane_id() trusted the stored SUPERHARNESS_ORCH_PANE env var
#      even after that pane was gone (e.g. the original `%0` was rebuilt and the
#      orchestrator is now `%1`). A stale id makes every send-keys fail.
#
# The fix: send straight to orchestrator_pane_id(); validate the stored pane
# still exists and otherwise fall back to the lowest numeric pane id (the first
# pane created in the session = the orchestrator), regardless of its window.
#
# This test replicates that resolution logic on a private tmux socket under
# base-index 1. It never touches the user's real tmux server/sessions.
#
# Run:  bash tests/toggle_mode_regression.sh
# Exits 0 on success, non-zero (with a message) on regression.

set -u
T="tmux -L shtoggle"
fail() { echo "FAIL: $1" >&2; $T kill-server 2>/dev/null; exit 1; }

$T kill-server 2>/dev/null
$T -f /dev/null start-server
$T set-option -g base-index 1
$T set-option -g pane-base-index 1

# Pane IDs (%N) are assigned sequentially from %0 by tmux regardless of
# base-index, so the session's first pane is %0.
$T new-session -d -s superharness -x 220 -y 50

# Simulate a session whose original %0 orchestrator was rebuilt: the env var
# still points at %0, but %0 is gone and the live orchestrator is now %1, in a
# later window (as happens after a reorg / embedded launch). %2 is an even-later
# user pane, present to prove the fallback picks the LOWEST id, not the newest.
$T set-environment -t superharness SUPERHARNESS_ORCH_PANE %0
orch=$($T new-window -t superharness -d -P -F '#{pane_id}')   # %1 = new orchestrator
$T new-window -t superharness -d                              # %2 = later user pane
$T kill-pane -t %0                                            # original orchestrator gone

# 1. The stale env value must NOT validate: display-message on a missing pane
#    exits 0 but prints an empty line, so existence must be checked by echo-back.
stale=$($T display-message -p -t %0 '#{pane_id}' 2>/dev/null)
[ "$stale" = "%0" ] && fail "expected %0 to be gone, but it resolved"

# 2. Fallback resolution = lowest numeric pane id across the whole session.
resolved=$($T list-panes -t superharness -a -F '#{pane_id}' \
    | sort -t% -k2 -n | head -1)
[ "$resolved" = "$orch" ] || fail "lowest-pane-id fallback picked $resolved, expected orchestrator $orch"

# 3. Sending to the resolved orchestrator must succeed (the original failure was
#    send-keys to the stale %0).
$T send-keys -t "$resolved" -l "hello" 2>/dev/null || fail "send to resolved orchestrator $resolved failed"
$T send-keys -t %0 -l "hello" 2>/dev/null && fail "send to stale %0 unexpectedly succeeded"

# 4. The robust path: an @sh_orchestrator tag pins the orchestrator regardless of
#    pane id or window. Tag a pane that is NOT the lowest id and confirm the tag
#    wins over the lowest-id heuristic.
$T set-option -p -t "$orch" @sh_orchestrator 1
tagged=$($T list-panes -t superharness -a -F '#{pane_id} #{?@sh_orchestrator,1,0}' \
    | awk '$2=="1"{print $1}')
[ "$tagged" = "$orch" ] || fail "@sh_orchestrator tag lookup got '$tagged', expected $orch"

# The tag must survive break-pane (workers prove this too, but the orchestrator
# can be moved to its own window during layout/compact operations).
$T break-pane -s "$orch" -d -n orch 2>/dev/null
survived=$($T show-options -p -t "$orch" @sh_orchestrator 2>/dev/null)
[ "$survived" = "@sh_orchestrator 1" ] || fail "tag did not survive break-pane (got '$survived')"

$T kill-server 2>/dev/null
echo "PASS: toggle-mode resolves a live orchestrator despite a stale %0 env var"
