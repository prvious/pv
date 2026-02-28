#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

pv link /tmp/e2e-static --name e2e-static
pv link /tmp/e2e-php --name e2e-php
pv link /tmp/e2e-laravel --name e2e-laravel
pv link /tmp/e2e-octane --name e2e-octane
pv link /tmp/e2e-php83 --name e2e-php83

echo "==> pv list"
OUTPUT=$(pv list)
echo "$OUTPUT"
assert_contains "$OUTPUT" "e2e-static" "e2e-static not in list"
assert_contains "$OUTPUT" "e2e-php" "e2e-php not in list"
assert_contains "$OUTPUT" "e2e-laravel" "e2e-laravel not in list"
assert_contains "$OUTPUT" "e2e-octane" "e2e-octane not in list"
assert_contains "$OUTPUT" "e2e-php83" "e2e-php83 not in list"
echo "$OUTPUT" | grep "e2e-octane" | grep -q "laravel-octane" || { echo "FAIL: octane type wrong"; exit 1; }
echo "$OUTPUT" | grep "e2e-php83" | grep -q "8.3" || { echo "FAIL: php83 version wrong"; exit 1; }

echo "==> pv php list (project associations)"
PHP_OUTPUT=$(pv php list)
echo "$PHP_OUTPUT"
echo "$PHP_OUTPUT" | grep "8.3" | grep -q "e2e-php83" || { echo "FAIL: php83 not associated with 8.3"; exit 1; }
echo "$PHP_OUTPUT" | grep "8.4" | grep -q "(default)" || { echo "FAIL: 8.4 not marked default"; exit 1; }

echo "==> Verify site configs"

# e2e-php83: reverse_proxy in sites/, php_server in sites-8.3/
echo "--- sites/e2e-php83.caddy ---"
cat ~/.pv/config/sites/e2e-php83.caddy
grep -q "reverse_proxy" ~/.pv/config/sites/e2e-php83.caddy || { echo "FAIL: php83 missing reverse_proxy"; exit 1; }
echo "--- sites-8.3/e2e-php83.caddy ---"
cat ~/.pv/config/sites-8.3/e2e-php83.caddy
grep -q "php_server" ~/.pv/config/sites-8.3/e2e-php83.caddy || { echo "FAIL: php83 missing php_server in sites-8.3"; exit 1; }

# e2e-octane: has frankenphp-worker.php in worker block
grep -q "frankenphp-worker.php" ~/.pv/config/sites/e2e-octane.caddy || { echo "FAIL: octane missing worker file"; exit 1; }

# e2e-static: has file_server
grep -q "file_server" ~/.pv/config/sites/e2e-static.caddy || { echo "FAIL: static missing file_server"; exit 1; }

echo "OK: all site configs verified"
