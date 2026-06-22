#!/usr/bin/env bash
#
# Regression test for the F5 status-bar task count (`task-counts` command).
#
# The status bar shows `F5:tasks(<completed>/<total>)`, where completed = tasks
# with status "done" in .superharness/tasks.json and total = all tasks. When
# there are no tasks it shows a bare `0` (rendered as `tasks(0)`).
#
# This drives the REAL binary's `task-counts` against real tasks.json files in
# both supported shapes (the `{ "tasks": [...] }` wrapper and a bare `[...]`
# array), asserting the printed string.
#
# ISOLATION: a throwaway $HOME (so no active_project.txt exists → the binary
# falls back to cwd) and throwaway project dirs. Never reads or writes the
# user's real superharness state, and never touches tmux at all.
#
# Run:  bash tests/task_counts_regression.sh

set -u
BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/superharness"
[ -x "$BIN" ] || { echo "FAIL: release binary not found at $BIN (run: cargo build --release)" >&2; exit 1; }

export HOME; HOME="$(mktemp -d /tmp/sh-tc-home.XXXXXX)"   # no active_project.txt -> cwd fallback
ROOT="$(mktemp -d /tmp/sh-tc.XXXXXX)"
cleanup(){ rm -rf "$HOME" "$ROOT"; }
trap cleanup EXIT
fail(){ echo "FAIL: $1" >&2; exit 1; }

# expect <name> <expected> <tasks.json-contents|__NONE__>
expect(){
  local name="$1" want="$2" body="$3"
  local proj="$ROOT/$name"
  mkdir -p "$proj/.superharness"
  [ "$body" = "__NONE__" ] || printf '%s' "$body" > "$proj/.superharness/tasks.json"
  local got
  got="$(cd "$proj" && "$BIN" task-counts 2>/dev/null)"
  if [ "$got" = "$want" ]; then
    echo "PASS  $name -> '$got'"
  else
    fail "$name: expected '$want', got '$got'"
  fi
}

# No tasks.json at all -> bare 0
expect no_file "0" "__NONE__"

# Empty wrapper -> bare 0
expect empty_wrapper "0" '{"tasks":[]}'

# 2 done of 4 (wrapper form)
expect two_of_four "2/4" '{"tasks":[
  {"id":"1","title":"a","status":"done"},
  {"id":"2","title":"b","status":"done"},
  {"id":"3","title":"c","status":"pending"},
  {"id":"4","title":"d","status":"blocked"}
]}'

# Tasks exist but none done -> 0/N (NOT bare 0)
expect none_done "0/3" '{"tasks":[
  {"id":"1","title":"a","status":"pending"},
  {"id":"2","title":"b","status":"in-progress"},
  {"id":"3","title":"c","status":"blocked"}
]}'

# All done
expect all_done "3/3" '{"tasks":[
  {"id":"1","title":"a","status":"done"},
  {"id":"2","title":"b","status":"done"},
  {"id":"3","title":"c","status":"done"}
]}'

# Legacy bare-array shape must also be accepted
expect bare_array "1/2" '[
  {"id":"1","title":"a","status":"done"},
  {"id":"2","title":"b","status":"pending"}
]'

# Large totals render fine
expect big "1/666" "$(python3 -c '
import json
t=[{"id":str(i),"title":"t","status":("done" if i==0 else "pending")} for i in range(666)]
print(json.dumps({"tasks":t}))')"

echo "PASS: task-counts renders completed/total correctly across all shapes"
