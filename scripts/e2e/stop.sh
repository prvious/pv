#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

sudo -E pv stop
sleep 2
STATUS=$(pv status)
echo "$STATUS"
assert_contains "$STATUS" "stopped" "server not stopped"
echo "OK: server stopped"
