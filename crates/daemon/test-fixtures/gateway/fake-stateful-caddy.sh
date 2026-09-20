#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  printf '%s\n' "$3" >> "${3%/*}/fake-validator-spawns.log"
  exit 0
fi

if [ "$1" = "run" ]; then
  parent_pid="${PPID:-}"
  if [ -z "$parent_pid" ] || [ "$parent_pid" = "1" ]; then
    exit 0
  fi
  child=""
  watcher=""
  watcher_stop_path="${3%/*}/fake-runtime-watcher-stop-$$"
  reaped_marker_path="${3%/*}/fake-runtime-reaped-$$"
  stop_watcher() {
    if [ -n "$watcher" ]; then
      if ! printf 'stop\n' > "$watcher_stop_path"; then
        kill "$watcher" 2>/dev/null || :
      fi
      wait "$watcher" 2>/dev/null || :
      watcher=""
    fi
  }
  publish_reaped() {
    printf 'reaped\n' > "$reaped_marker_path"
  }
  # shellcheck disable=SC2329 # Invoked indirectly by the signal trap.
  stop_child() {
    if [ -n "$child" ]; then
      kill "$child" 2>/dev/null || :
      wait "$child" 2>/dev/null || :
      child=""
    fi
    stop_watcher
    publish_reaped
    exit 0
  }
  trap 'stop_child' TERM INT
  watch_parent() {
    while [ ! -f "$watcher_stop_path" ]; do
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
  PV_FAKE_RUNTIME=caddy python3 - "$3" < "$0.server.py" &
  child="$!"
  if wait "$child"; then
    child_status=0
  else
    child_status="$?"
  fi
  child=""
  stop_watcher
  publish_reaped
  exit "$child_status"
fi

exit 2
