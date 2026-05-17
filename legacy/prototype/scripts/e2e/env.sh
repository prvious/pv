#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# ── 1. pv env produces output ─────────────────────────────────────────
echo "==> pv env produces output"
ENV_OUT=$(pv env)
echo "$ENV_OUT"
[ -n "$ENV_OUT" ] || { echo "FAIL: pv env produced empty output"; exit 1; }

# ── 2. Output contains ~/.pv/bin ───────────────────────────────────────
echo "==> Output contains ~/.pv/bin"
assert_contains "$ENV_OUT" ".pv/bin" "pv env output missing .pv/bin"

# ── 3. Output contains ~/.pv/composer/vendor/bin ───────────────────────
echo "==> Output contains ~/.pv/composer/vendor/bin"
assert_contains "$ENV_OUT" ".pv/composer/vendor/bin" "pv env output missing .pv/composer/vendor/bin"

# ── 4. Output is valid shell syntax ───────────────────────────────────
echo "==> Output is valid shell syntax (can be eval'd)"
eval "$ENV_OUT"

# ── 5. After eval, php resolves to ~/.pv/bin/php ──────────────────────
echo "==> php resolves via PATH after eval"
PHP_PATH=$(command -v php)
echo "  php -> $PHP_PATH"
assert_contains "$PHP_PATH" ".pv/bin/php" "php did not resolve to ~/.pv/bin/php"

# ── 6. After eval, composer resolves to ~/.pv/bin/composer ─────────────
echo "==> composer resolves via PATH after eval"
COMPOSER_PATH=$(command -v composer)
echo "  composer -> $COMPOSER_PATH"
assert_contains "$COMPOSER_PATH" ".pv/bin/composer" "composer did not resolve to ~/.pv/bin/composer"

# ── 7. PATH-resolved php actually works ───────────────────────────────
echo "==> php --version works via PATH"
PHP_VER=$(php --version | head -1)
echo "  $PHP_VER"
assert_contains "$PHP_VER" "PHP 8" "php --version did not return expected output"

# ── 8. PATH-resolved composer actually works ──────────────────────────
echo "==> composer --version works via PATH"
COMPOSER_VER=$(composer --version 2>&1 | head -1)
echo "  $COMPOSER_VER"
assert_contains "$COMPOSER_VER" "Composer" "composer --version did not return expected output"

# ── 9. pv env with SHELL=fish produces fish syntax ────────────────────
echo "==> pv env with SHELL=/usr/bin/fish produces fish syntax"
FISH_OUT=$(SHELL=/usr/bin/fish pv env)
echo "$FISH_OUT"
assert_contains "$FISH_OUT" "fish_add_path" "fish output missing fish_add_path"

# ── 10. pv env with SHELL=bash produces export syntax ─────────────────
echo "==> pv env with SHELL=/bin/bash produces export syntax"
BASH_OUT=$(SHELL=/bin/bash pv env)
echo "$BASH_OUT"
assert_contains "$BASH_OUT" "export PATH=" "bash output missing export PATH="

# ── 11. pv env with SHELL=zsh produces export syntax ──────────────────
echo "==> pv env with SHELL=/bin/zsh produces export syntax"
ZSH_OUT=$(SHELL=/bin/zsh pv env)
echo "$ZSH_OUT"
assert_contains "$ZSH_OUT" "export PATH=" "zsh output missing export PATH="

echo ""
echo "OK: pv env verified"
