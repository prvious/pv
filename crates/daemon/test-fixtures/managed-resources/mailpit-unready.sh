#!/bin/sh
set -eu

stop() {
  exit 0
}

trap stop TERM INT

while true; do
  sleep 1
done
