#!/usr/bin/env bash
#
# Regression test for the F5 tasks modal hiding tasks with unrecognized statuses.
#
# The orchestrator writes .superharness/tasks.json freeform. When a task's status
# wasn't one of the hardcoded set (in-progress/pending/blocked/done/cancelled) it
# counted toward the total but appeared in NO group and NO per-status tally — so
# the modal showed e.g. "Tasks: 7 | ... done:0 ..." with zero task bodies (the
# real bug: 7 tasks all status "completed", all invisible).
#
# The fix routes status through canonical normalization (so "completed" -> done,
# "in_progress" -> in-progress) and adds an "OTHER" group so a genuinely unknown
# status is still displayed. This test drives the REAL `tasks-modal` and asserts
# every task body is present and the tally is correct.
#
# ISOLATION: throwaway $HOME (no active_project.txt -> cwd fallback) + throwaway
# project. No tmux, no user state touched.
#
# Run:  bash tests/tasks_modal_regression.sh

set -u
BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/superharness"
[ -x "$BIN" ] || { echo "FAIL: release binary not found at $BIN (run: cargo build --release)" >&2; exit 1; }

export HOME; HOME="$(mktemp -d /tmp/sh-tm-home.XXXXXX)"
PROJ="$(mktemp -d /tmp/sh-tm.XXXXXX)"; mkdir -p "$PROJ/.superharness"
cleanup(){ rm -rf "$HOME" "$PROJ"; }
trap cleanup EXIT
fail(){ echo "FAIL: $1" >&2; exit 1; }

# Synonym (completed -> done), underscore (in_progress -> in-progress), and a
# genuinely unknown status that must still show under OTHER.
cat > "$PROJ/.superharness/tasks.json" <<'JSON'
{"tasks":[
  {"id":"t1","title":"FINISHED ALPHA","description":"d","status":"completed","priority":"high"},
  {"id":"t2","title":"WORKING BETA","description":"d","status":"in_progress","priority":"medium"},
  {"id":"t3","title":"WEIRD GAMMA","description":"d","status":"frobnicating","priority":"low"}
]}
JSON

out="$(cd "$PROJ" && "$BIN" tasks-modal 2>&1)"

# Every task body must be visible — none hidden.
for title in "FINISHED ALPHA" "WORKING BETA" "WEIRD GAMMA"; do
  printf '%s' "$out" | grep -qF "$title" || { echo "$out"; fail "task '$title' is not displayed (hidden by status)"; }
done

# Tally must fold synonyms: 1 done (completed), 1 in-progress (in_progress), 1 other.
printf '%s' "$out" | grep -qE "Tasks:.* 3 " || fail "expected total of 3 tasks"
printf '%s' "$out" | grep -qE "done:1"        || { echo "$out"; fail "'completed' should tally as done:1"; }
printf '%s' "$out" | grep -qE "in-progress:1" || fail "'in_progress' should tally as in-progress:1"
printf '%s' "$out" | grep -qE "other:1"        || fail "unknown status should tally as other:1"

# The unknown status must be surfaced with its RAW text so the user sees it.
printf '%s' "$out" | grep -qiF "frobnicating" || fail "raw unknown status 'frobnicating' should be shown"

echo "PASS: tasks modal displays synonym + unknown-status tasks; tally folds synonyms correctly"
