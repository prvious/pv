#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)
# shellcheck source=helpers.sh
source "$SCRIPT_DIR/helpers.sh"

PHP_VERSION="${PHP_VERSION:-8.4}"
ETC_DIR="$HOME/.pv/php/$PHP_VERSION/etc"
CONFD_DIR="$HOME/.pv/php/$PHP_VERSION/conf.d"

echo "==> Verify per-version ini layout for PHP $PHP_VERSION"
test -d "$ETC_DIR"   || { echo "FAIL: $ETC_DIR missing"; exit 1; }
test -d "$CONFD_DIR" || { echo "FAIL: $CONFD_DIR missing"; exit 1; }
test -s "$ETC_DIR/php.ini" || { echo "FAIL: $ETC_DIR/php.ini missing or empty"; exit 1; }
test -s "$ETC_DIR/php.ini-development" || { echo "FAIL: $ETC_DIR/php.ini-development missing"; exit 1; }
test -s "$CONFD_DIR/00-pv.ini" || { echo "FAIL: $CONFD_DIR/00-pv.ini missing"; exit 1; }
echo "  OK: layout present"

echo "==> Verify CLI loads the per-version ini"
LOADED=$(php -r 'echo php_ini_loaded_file();')
echo "  Loaded ini: $LOADED"
if [ "$LOADED" != "$ETC_DIR/php.ini" ]; then
    echo "FAIL: php loaded $LOADED, expected $ETC_DIR/php.ini"
    exit 1
fi

echo "==> Verify CLI scans the per-version conf.d"
SCANNED=$(php -r 'echo php_ini_scanned_files();')
echo "  Scanned: $SCANNED"
echo "$SCANNED" | grep -q "00-pv.ini" || { echo "FAIL: 00-pv.ini not in scanned files"; exit 1; }

echo "==> Verify 00-pv.ini sets session.save_path under ~/.pv/data/sessions"
SAVE_PATH=$(php -r 'echo ini_get("session.save_path");')
echo "  session.save_path = $SAVE_PATH"
EXPECTED_SAVE_PATH="$HOME/.pv/data/sessions/$PHP_VERSION"
if [ "$SAVE_PATH" != "$EXPECTED_SAVE_PATH" ]; then
    echo "FAIL: session.save_path = $SAVE_PATH, want $EXPECTED_SAVE_PATH"
    exit 1
fi

echo "==> Drop a 99-local.ini and verify it overrides"
echo 'memory_limit = 42M' > "$CONFD_DIR/99-local.ini"
GOT=$(php -r 'echo ini_get("memory_limit");')
if [ "$GOT" != "42M" ]; then
    echo "FAIL: memory_limit = $GOT, want 42M (99-local.ini override didn't apply)"
    rm -f "$CONFD_DIR/99-local.ini"
    exit 1
fi
echo "  OK: 99-local.ini override applied"
rm -f "$CONFD_DIR/99-local.ini"

echo "OK: php-ini phase passed"

# Note: "user edits survive reinstall" and "00-pv.ini regenerated on
# reinstall" are covered by phpenv unit tests in Task 3; replicating them
# here would require re-running the full install which is slow in CI and
# adds little signal beyond the unit tests.
