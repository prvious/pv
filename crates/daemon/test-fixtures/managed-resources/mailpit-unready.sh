#!/bin/sh
set -eu

parent_pid="$PPID"
if [ "$parent_pid" -eq 1 ]; then
  exit 0
fi

stop() {
  exit 0
}

trap stop TERM INT

while true; do
  current_parent_pid="$(ps -o ppid= -p "$$" 2>/dev/null | tr -d '[:space:]')"
  if [ "$current_parent_pid" != "$parent_pid" ]; then
    exit 0
  fi
  sleep 0.1
done
