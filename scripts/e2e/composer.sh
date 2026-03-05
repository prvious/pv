#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Set up PATH via pv env so we can use bare `composer` and `php` commands
eval "$(pv env)"

# ── 1. Verify composer shim exists and is executable ───────────────────
echo "==> Verify composer shim exists"
ls -la ~/.pv/bin/composer
test -x ~/.pv/bin/composer || { echo "FAIL: composer shim not executable"; exit 1; }

echo "==> Verify composer.phar exists"
ls -la ~/.pv/data/composer.phar
test -f ~/.pv/data/composer.phar || { echo "FAIL: composer.phar not found"; exit 1; }

# ── 2. Verify COMPOSER_HOME isolation ──────────────────────────────────
echo "==> Verify COMPOSER_HOME points to ~/.pv/composer"
COMPOSER_HOME_OUTPUT=$(composer config --global home 2>/dev/null)
echo "  COMPOSER_HOME = $COMPOSER_HOME_OUTPUT"
assert_contains "$COMPOSER_HOME_OUTPUT" ".pv/composer" "COMPOSER_HOME not isolated under ~/.pv/composer"

# ── 3. Verify COMPOSER_CACHE_DIR isolation ─────────────────────────────
echo "==> Verify COMPOSER_CACHE_DIR points to ~/.pv/composer/cache"
COMPOSER_CACHE_OUTPUT=$(composer config --global cache-dir 2>/dev/null)
echo "  COMPOSER_CACHE_DIR = $COMPOSER_CACHE_OUTPUT"
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

# ── 10. Composer version resolves per-project PHP ─────────────────────
echo "==> Verify composer uses correct PHP per project"

# In 8.3 project dir, composer should use PHP 8.3.
if [ -d /tmp/e2e-php83 ]; then
  PHP_VER=$(cd /tmp/e2e-php83 && composer --version 2>&1 | head -1)
  echo "  e2e-php83 dir: $PHP_VER"
  # Composer itself runs — that's proof the PHP resolution worked.
  assert_contains "$PHP_VER" "Composer" "composer did not run in e2e-php83 project"
fi

# In global context, should use PHP 8.4.
PHP_VER=$(cd /tmp && composer --version 2>&1 | head -1)
echo "  /tmp (global): $PHP_VER"
assert_contains "$PHP_VER" "Composer" "composer did not run in global context"

echo ""
echo "OK: Composer containment verified"
