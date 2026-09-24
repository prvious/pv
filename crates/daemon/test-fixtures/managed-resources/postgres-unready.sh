#!/bin/sh
set -eu

parent_pid="$PPID"
if [ -n "${PV_TEST_PARENT_CAPTURE_MARKER:-}" ] && [ -n "${PV_TEST_PARENT_CAPTURE_RELEASE:-}" ]; then
  printf 'started\n' > "$PV_TEST_PARENT_CAPTURE_MARKER"
  while [ ! -f "$PV_TEST_PARENT_CAPTURE_RELEASE" ]; do
    current_parent_pid="$(ps -o ppid= -p "$$" 2>/dev/null | tr -d '[:space:]')"
    if [ -z "$current_parent_pid" ] || [ "$current_parent_pid" = "1" ] || [ "$current_parent_pid" != "$parent_pid" ]; then
      exit 0
    fi
    sleep 0.1
  done
fi
if [ "$parent_pid" -eq 1 ]; then
  exit 0
fi
data_dir=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -D)
      data_dir="$2"
      shift 2
      ;;
    -h|-p)
      shift 2
      ;;
    *)
      echo "unexpected postgres argument: $1" >&2
      exit 64
      ;;
  esac
done

if [ -z "$data_dir" ] || [ ! -f "$data_dir/PG_VERSION" ]; then
  echo "postgres data dir is not initialized" >&2
  exit 64
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
