#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  PV_FAKE_RUNTIME=caddy python3 - "$3" < "$0.server.py" &
  child="$!"
  trap 'kill "$child" 2>/dev/null || :; wait "$child" 2>/dev/null || :; exit 0' TERM INT
  wait "$child"
  exit "$?"
fi

exit 2
