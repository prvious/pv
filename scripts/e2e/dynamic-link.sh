#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

sudo -E pv link /tmp/e2e-dynamic --name e2e-dynamic
sleep 3

setup_curl

curl_site "e2e-dynamic.test" "dynamic works"
echo "OK: dynamic site (linked while server running, no restart needed)"
