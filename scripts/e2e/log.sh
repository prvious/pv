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

echo "==> Verify Caddy-managed log files exist"
ls -la ~/.pv/logs/caddy.log
ls -la ~/.pv/logs/caddy-8.3.log
echo "OK: both Caddy log files exist"

echo "==> Verify stderr log files exist"
ls -la ~/.pv/logs/caddy-stderr.log
ls -la ~/.pv/logs/caddy-8.3-stderr.log
echo "OK: both stderr log files exist"

echo "==> Verify main Caddyfile has log rotation"
grep -q "log {" ~/.pv/config/Caddyfile || { echo "FAIL: Caddyfile missing log directive"; exit 1; }
grep -q "roll_size 10MiB" ~/.pv/config/Caddyfile || { echo "FAIL: Caddyfile missing roll_size"; exit 1; }
grep -q "roll_keep 3" ~/.pv/config/Caddyfile || { echo "FAIL: Caddyfile missing roll_keep"; exit 1; }
echo "OK: main Caddyfile has log rotation config"

echo "==> Verify version Caddyfile has log rotation"
grep -q "log {" ~/.pv/config/php-8.3.Caddyfile || { echo "FAIL: version Caddyfile missing log directive"; exit 1; }
grep -q "roll_size 10MiB" ~/.pv/config/php-8.3.Caddyfile || { echo "FAIL: version Caddyfile missing roll_size"; exit 1; }
echo "OK: version Caddyfile has log rotation config"
