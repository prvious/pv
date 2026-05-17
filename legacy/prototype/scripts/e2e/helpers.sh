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

# strip_ansi removes ANSI escape codes from text.
# lipgloss v2 always emits ANSI codes even when output is piped/captured.
strip_ansi() {
  local esc=$'\x1b'
  sed "s/${esc}\[[0-9;]*m//g"
}

# assert_contains TEXT PATTERN MSG — grep TEXT for PATTERN or fail with MSG.
assert_contains() {
  local text="$1"
  local pattern="$2"
  local msg="$3"
  echo "$text" | strip_ansi | grep -q "$pattern" || { echo "FAIL: $msg"; exit 1; }
}

# assert_fails CMD... — run CMD, expect non-zero exit.
assert_fails() {
  if "$@" 2>&1; then
    echo "FAIL: expected failure from: $*"
    exit 1
  fi
}

# wait_for_tcp HOST PORT [TIMEOUT_SEC]
# Returns 0 once HOST:PORT accepts a TCP connection, or fails after TIMEOUT.
# Used by binary-service e2e phases to gate on supervisor readiness.
wait_for_tcp() {
  local host="$1"
  local port="$2"
  local timeout="${3:-30}"
  local i=0
  while ! nc -z "$host" "$port" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -ge "$timeout" ]; then
      echo "wait_for_tcp: ${host}:${port} not accepting after ${timeout}s" >&2
      return 1
    fi
    sleep 1
  done
}
