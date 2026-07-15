#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  python3 - "$3" < "$0.server.py" &
  child="$!"
  trap ':' USR1
  trap 'kill "$child"; wait "$child"; exit 0' TERM INT
  while true; do
    wait "$child" && exit 0
    status="$?"
    if kill -0 "$child" 2>/dev/null; then
      continue
    fi
    exit "$status"
  done
fi

exit 2
