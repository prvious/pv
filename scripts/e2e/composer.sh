#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Set up PATH via pv env so we can use bare `composer` and `php` commands
eval "$(pv env)"

# ── 1. Verify composer symlink exists and points to composer.phar ──────
echo "==> Verify composer symlink exists"
ls -la ~/.pv/bin/composer
test -L ~/.pv/bin/composer || { echo "FAIL: composer is not a symlink"; exit 1; }

echo "==> Verify composer.phar exists and is executable"
ls -la ~/.pv/internal/bin/composer.phar
test -f ~/.pv/internal/bin/composer.phar || { echo "FAIL: composer.phar not found"; exit 1; }
test -x ~/.pv/internal/bin/composer.phar || { echo "FAIL: composer.phar not executable"; exit 1; }

# ── 2. Verify COMPOSER_HOME isolation via pv env ──────────────────────
echo "==> Verify COMPOSER_HOME is set by pv env"
echo "  COMPOSER_HOME = $COMPOSER_HOME"
assert_contains "$COMPOSER_HOME" ".pv/composer" "COMPOSER_HOME not isolated under ~/.pv/composer"

echo "==> Verify composer config --global home"
COMPOSER_HOME_OUTPUT=$(composer config --global home 2>&1)
echo "  composer config home = $COMPOSER_HOME_OUTPUT"
assert_contains "$COMPOSER_HOME_OUTPUT" ".pv/composer" "COMPOSER_HOME not isolated under ~/.pv/composer"

# ── 3. Verify COMPOSER_CACHE_DIR isolation via pv env ─────────────────
echo "==> Verify COMPOSER_CACHE_DIR is set by pv env"
echo "  COMPOSER_CACHE_DIR = $COMPOSER_CACHE_DIR"
assert_contains "$COMPOSER_CACHE_DIR" ".pv/composer/cache" "COMPOSER_CACHE_DIR not isolated under ~/.pv/composer/cache"

echo "==> Verify composer config --global cache-dir"
COMPOSER_CACHE_OUTPUT=$(composer config --global cache-dir 2>&1)
echo "  composer config cache-dir = $COMPOSER_CACHE_OUTPUT"
assert_contains "$COMPOSER_CACHE_OUTPUT" ".pv/composer/cache" "COMPOSER_CACHE_DIR not isolated under ~/.pv/composer/cache"

# ── 4. Verify nothing touches ~/.composer ──────────────────────────────
echo "==> Verify ~/.composer is not created"
if [ -d "$HOME/.composer" ]; then
  echo "FAIL: ~/.composer exists — isolation is broken"
  ls -la "$HOME/.composer"
  exit 1
fi
echo "  OK: ~/.composer does not exist"

# ── 5. Composer global require — real install to disk ──────────────────
echo "==> composer global require laravel/installer"
composer global require laravel/installer --no-interaction 2>&1 | tail -5

echo "==> Verify laravel/installer landed in ~/.pv/composer/vendor"
test -d ~/.pv/composer/vendor/laravel/installer || { echo "FAIL: laravel/installer not in ~/.pv/composer/vendor"; exit 1; }
echo "  OK: ~/.pv/composer/vendor/laravel/installer exists"

echo "==> Verify laravel binary in ~/.pv/composer/vendor/bin"
test -f ~/.pv/composer/vendor/bin/laravel || { echo "FAIL: laravel binary not in ~/.pv/composer/vendor/bin"; exit 1; }
echo "  OK: ~/.pv/composer/vendor/bin/laravel exists"

# ── 6. Verify the binary is actually executable via PATH ──────────────
echo "==> Run laravel --version via PATH (composer/vendor/bin is in PATH)"
LARAVEL_OUT=$(laravel --version 2>&1 || true)
echo "  $LARAVEL_OUT"
assert_contains "$LARAVEL_OUT" "Laravel Installer" "laravel binary did not produce expected output"

# ── 7. Verify cache was populated ─────────────────────────────────────
echo "==> Verify composer cache was populated under ~/.pv/composer/cache"
CACHE_COUNT=$(find ~/.pv/composer/cache -type f 2>/dev/null | wc -l | tr -d ' ')
echo "  Cache files: $CACHE_COUNT"
[ "$CACHE_COUNT" -gt 0 ] || { echo "FAIL: composer cache is empty"; exit 1; }

# ── 8. Still no ~/.composer ───────────────────────────────────────────
echo "==> Final check: ~/.composer still does not exist"
if [ -d "$HOME/.composer" ]; then
  echo "FAIL: ~/.composer appeared after composer global require"
  ls -la "$HOME/.composer"
  exit 1
fi

# ── 9. Composer global remove — cleanup works in isolation ─────────────
echo "==> composer global remove laravel/installer"
composer global remove laravel/installer --no-interaction 2>&1 | tail -3

echo "==> Verify laravel/installer removed from vendor"
if [ -d ~/.pv/composer/vendor/laravel/installer ]; then
  echo "FAIL: laravel/installer still in vendor after remove"
  exit 1
fi
echo "  OK: laravel/installer removed"

# ── 10. Verify composer runs correctly ─────────────────────────────────
echo "==> Verify composer --version works"
PHP_VER=$(composer --version 2>&1 | head -1)
echo "  $PHP_VER"
assert_contains "$PHP_VER" "Composer" "composer did not produce expected version output"

echo ""
echo "OK: Composer containment verified"
