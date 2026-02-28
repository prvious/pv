#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> pv log -n 5"
OUTPUT=$(pv log -n 5)
if [ -z "$OUTPUT" ]; then
  echo "FAIL: log output is empty"
  exit 1
fi
echo "$OUTPUT"
echo "OK: pv log returns output"

echo "==> Verify log files exist"
ls -la ~/.pv/logs/caddy.log
ls -la ~/.pv/logs/caddy-8.3.log
echo "OK: both log files exist"
