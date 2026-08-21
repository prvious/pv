#!/bin/sh
set -eu

if [ "$1" = "validate" ]; then
  test -f "$3"
  exit 0
fi

if [ "$1" = "run" ]; then
  trap 'exit 0' TERM INT
  while true; do
    sleep 1
  done
fi

exit 2
