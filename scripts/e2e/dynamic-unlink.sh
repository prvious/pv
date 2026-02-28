#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

sudo -E pv unlink e2e-static
sleep 2

# Verify removed from list
if pv list | grep -q "e2e-static"; then
  echo "FAIL: e2e-static still in list after unlink"
  exit 1
fi
echo "OK: e2e-static removed from list"

# Verify site no longer serves
setup_curl
if curl -sf --cacert "$CACERT" $RESOLVE https://e2e-static.test/ 2>/dev/null; then
  echo "FAIL: e2e-static still serving after unlink"
  exit 1
fi
echo "OK: e2e-static no longer served"
