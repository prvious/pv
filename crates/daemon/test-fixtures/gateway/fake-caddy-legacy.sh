#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  parent_pid="${PPID:-}"
  if [ -z "$parent_pid" ] || [ "$parent_pid" = "1" ]; then
    exit 0
  fi
  child=""
  watcher=""
  # shellcheck disable=SC2329 # Invoked indirectly by the signal trap.
  stop_child() {
    if [ -n "$child" ]; then
      kill '%?python3' 2>/dev/null || :
      wait "$child" 2>/dev/null || :
      child=""
    fi
    if [ -n "$watcher" ]; then
      kill '%?watch_parent' 2>/dev/null || :
      wait "$watcher" 2>/dev/null || :
      watcher=""
    fi
    exit 0
  }
  trap 'stop_child' TERM INT
  watch_parent() {
    while true; do
      current_parent_pid="$(ps -o ppid= -p "$$" 2>/dev/null | tr -d '[:space:]')"
      if [ -z "$current_parent_pid" ] || [ "$current_parent_pid" = "1" ] || [ "$current_parent_pid" != "$parent_pid" ]; then
        kill -TERM "$$" 2>/dev/null || :
        return 0
      fi
      sleep 0.1
    done
  }
  watch_parent &
  watcher="$!"
  export PV_FAKE_FIXTURE_PARENT_PID="$parent_pid"
  python3 "$0.server.py" "$3" &
  child="$!"
  if wait "$child"; then
    child_status=0
  else
    child_status="$?"
  fi
  child=""
  kill '%?watch_parent' 2>/dev/null || :
  wait "$watcher" 2>/dev/null || :
  watcher=""
  exit "$child_status"
fi

exit 2
