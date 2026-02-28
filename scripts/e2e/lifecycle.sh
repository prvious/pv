#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Switch global to 8.3
pv use php:8.3
echo "==> Verify settings after switching to 8.3"
grep -q '"global_php": "8.3"' ~/.pv/config/settings.json || { echo "FAIL: settings not updated"; exit 1; }
readlink ~/.pv/bin/frankenphp | grep -q "8.3" || { echo "FAIL: symlink not pointing to 8.3"; exit 1; }
OUT=$(cd /tmp && ~/.pv/bin/php --version)
echo "$OUT"
echo "$OUT" | grep -qi "8\.3" || { echo "FAIL: shim not resolving to 8.3"; exit 1; }
echo "OK: pv use php:8.3 works"

# Switch back to 8.4
pv use php:8.4
grep -q '"global_php": "8.4"' ~/.pv/config/settings.json || { echo "FAIL: settings not updated back"; exit 1; }
echo "OK: pv use php:8.4 works"

# Unlink e2e-php83 to free PHP 8.3
sudo -E pv unlink e2e-php83
if pv list | grep -q "e2e-php83"; then
  echo "FAIL: e2e-php83 still in list"; exit 1
fi
echo "OK: e2e-php83 unlinked"

# Remove PHP 8.3
pv php remove 8.3
if [ -d ~/.pv/php/8.3 ]; then
  echo "FAIL: PHP 8.3 directory still exists"; exit 1
fi
PHP_OUT=$(pv php list)
echo "$PHP_OUT"
if echo "$PHP_OUT" | grep -qE "8\.3"; then
  echo "FAIL: 8.3 still in php list"; exit 1
fi
echo "OK: PHP 8.3 removed"
