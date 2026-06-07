#!/bin/sh
set -eu

artifact_root=$1
php_binary=${PV_COMPOSER_SMOKE_PHP:-}
expected_version=${PV_UPSTREAM_VERSION:-}
[ -n "$php_binary" ] || {
  printf '%s\n' "composer smoke skipped: PV_COMPOSER_SMOKE_PHP is not set" >&2
  exit 0
}
[ -n "$expected_version" ] || {
  printf '%s\n' "PV_UPSTREAM_VERSION is required for Composer smoke" >&2
  exit 42
}

[ -f "$artifact_root/composer.phar" ] || {
  printf '%s\n' "missing composer.phar in $artifact_root" >&2
  exit 42
}

tmp_output=$(mktemp "${TMPDIR:-/tmp}/pv-composer-smoke.XXXXXX")
trap 'rm -f "$tmp_output"' 0
"$php_binary" "$artifact_root/composer.phar" --version >"$tmp_output"
grep -F "Composer version $expected_version" "$tmp_output" >/dev/null
