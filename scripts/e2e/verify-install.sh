#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Set up PATH so php shim and composer symlink are available.
eval "$(pv env)"

echo "==> Verify 8.4 binaries"
ls -la ~/.pv/php/8.4/frankenphp
ls -la ~/.pv/php/8.4/php

echo "==> Verify 8.3 binaries"
ls -la ~/.pv/php/8.3/frankenphp
ls -la ~/.pv/php/8.3/php

echo "==> pv php:list"
OUTPUT=$(pv php:list 2>&1)
echo "$OUTPUT"
assert_contains "$OUTPUT" "(default)" "8.4 not marked as default"
assert_contains "$OUTPUT" "8.3" "8.3 not listed"

echo "==> Verify frankenphp symlink points to 8.4"
readlink ~/.pv/bin/frankenphp | grep -q "8.4" || { echo "FAIL: symlink not pointing to 8.4"; exit 1; }

echo "==> Verify php shim works"
~/.pv/bin/php --version

echo "==> Verify settings.json"
cat ~/.pv/config/settings.json
grep -q '"global_php": "8.4"' ~/.pv/config/settings.json || { echo "FAIL: settings.json wrong"; exit 1; }

echo "==> Verify resolver"
cat /etc/resolver/test

echo "==> Verify composer symlink and phar"
test -L ~/.pv/bin/composer || { echo "FAIL: composer is not a symlink"; exit 1; }
test -f ~/.pv/internal/bin/composer.phar || { echo "FAIL: composer.phar not found"; exit 1; }

echo "==> Verify composer directories"
test -d ~/.pv/composer || { echo "FAIL: ~/.pv/composer not created"; exit 1; }
test -d ~/.pv/composer/cache || { echo "FAIL: ~/.pv/composer/cache not created"; exit 1; }

echo "==> Verify composer runs"
~/.pv/bin/composer --version

echo "OK: both PHP versions and composer verified"
