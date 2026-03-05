#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Set up PATH via pv env so we can use bare `php` command
eval "$(pv env)"

echo "==> Shim in e2e-php83 dir (.pv-php -> 8.3)"
OUT=$(cd /tmp/e2e-php83 && php --version)
echo "$OUT"
echo "$OUT" | grep -qi "8\.3" || { echo "FAIL: shim did not resolve to 8.3"; exit 1; }

echo "==> Shim in e2e-php dir (composer.json -> 8.4)"
OUT=$(cd /tmp/e2e-php && php --version)
echo "$OUT"
echo "$OUT" | grep -qi "8\.4" || { echo "FAIL: shim did not resolve to 8.4"; exit 1; }

echo "==> Shim in /tmp (global fallback -> 8.4)"
OUT=$(cd /tmp && php --version)
echo "$OUT"
echo "$OUT" | grep -qi "8\.4" || { echo "FAIL: shim did not resolve to global 8.4"; exit 1; }

echo "OK: PHP shim resolution works"
