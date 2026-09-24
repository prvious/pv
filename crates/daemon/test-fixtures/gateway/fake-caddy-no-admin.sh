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
  watcher=""
  # shellcheck disable=SC2329 # Invoked indirectly by the signal trap.
  stop_fixture() {
    if [ -n "$watcher" ]; then
      kill '%?watch_parent' 2>/dev/null || :
      wait "$watcher" 2>/dev/null || :
    fi
    exit 0
  }
  trap 'stop_fixture' TERM INT
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
  wait "$watcher" || :
fi

exit 2
