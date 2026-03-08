#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Set up PATH via pv env so we can use bare `php` command
eval "$(pv env)"

echo "==> Shim in e2e-php83 dir (pv.yml -> 8.3)"
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

# ── php:current command tests ──────────────────────────────────

echo "==> php:current in pv.yml dir (8.3)"
OUT=$(cd /tmp/e2e-php83 && pv php:current)
echo "$OUT"
[ "$OUT" = "8.3" ] || { echo "FAIL: php:current should be 8.3, got '$OUT'"; exit 1; }

echo "==> php:current in composer.json dir (8.4)"
OUT=$(cd /tmp/e2e-php && pv php:current)
echo "$OUT"
[ "$OUT" = "8.4" ] || { echo "FAIL: php:current should be 8.4, got '$OUT'"; exit 1; }

echo "==> php:current in /tmp (global fallback 8.4)"
OUT=$(cd /tmp && pv php:current)
echo "$OUT"
[ "$OUT" = "8.4" ] || { echo "FAIL: php:current should be 8.4, got '$OUT'"; exit 1; }

echo "==> php:current walks up from subdirectory"
mkdir -p /tmp/e2e-php83/src/Models
OUT=$(cd /tmp/e2e-php83/src/Models && pv php:current)
echo "$OUT"
[ "$OUT" = "8.3" ] || { echo "FAIL: php:current should walk up to 8.3, got '$OUT'"; exit 1; }

echo "==> php:current pv.yml beats composer.json"
# e2e-php83 has both pv.yml (8.3) and composer.json (^8.0) — pv.yml should win
OUT=$(cd /tmp/e2e-php83 && pv php:current)
echo "$OUT"
[ "$OUT" = "8.3" ] || { echo "FAIL: pv.yml should take priority, got '$OUT'"; exit 1; }

echo "OK: PHP shim and php:current resolution works"
