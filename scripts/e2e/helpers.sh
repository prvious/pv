#!/usr/bin/env bash
set -euo pipefail

# Shared helpers for E2E test scripts.
# Source this file: source "$(dirname "$0")/helpers.sh"

# setup_curl sets CACERT and RESOLVE for all .test domains.
setup_curl() {
  CACERT="${HOME}/.pv/caddy/pki/authorities/local/root.crt"
  RESOLVE="--resolve e2e-static.test:443:127.0.0.1 --resolve e2e-php.test:443:127.0.0.1 --resolve e2e-laravel.test:443:127.0.0.1 --resolve e2e-octane.test:443:127.0.0.1 --resolve e2e-php83.test:443:127.0.0.1 --resolve e2e-dynamic.test:443:127.0.0.1"
  export CACERT RESOLVE
}

# curl_site DOMAIN TEXT — curl the domain and grep for TEXT (with retries).
curl_site() {
  local domain="$1"
  local text="$2"
  local i
  for i in 1 2 3; do
    if curl -sf --max-time 5 --cacert "$CACERT" $RESOLVE "https://${domain}/" 2>/dev/null | grep -q "$text"; then
      echo "OK: ${domain}"
      return 0
    fi
    [ "$i" -lt 3 ] && sleep 2
  done
  echo "FAIL: ${domain} did not return expected text: ${text}"
  curl -v --max-time 5 --cacert "$CACERT" $RESOLVE "https://${domain}/" 2>&1 || true
  exit 1
}

# assert_contains TEXT PATTERN MSG — grep TEXT for PATTERN or fail with MSG.
assert_contains() {
  local text="$1"
  local pattern="$2"
  local msg="$3"
  echo "$text" | grep -q "$pattern" || { echo "FAIL: $msg"; exit 1; }
}

# assert_fails CMD... — run CMD, expect non-zero exit.
assert_fails() {
  if "$@" 2>&1; then
    echo "FAIL: expected failure from: $*"
    exit 1
  fi
}
