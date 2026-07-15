#!/bin/sh
set -eu

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
  sleep 1
done
