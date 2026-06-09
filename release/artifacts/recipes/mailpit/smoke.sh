#!/bin/sh
set -eu

artifact_root=$1
mailpit_binary="$artifact_root/bin/mailpit"
expected_version=${PV_UPSTREAM_VERSION:-}
pid=
tmp_dir=

need() {
  command -v "$1" >/dev/null 2>&1 || {
    printf '%s\n' "missing required command: $1" >&2
    exit 42
  }
}

available_port() {
  python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

check_tcp_bind() {
  port=$1
  python3 - "$port" <<'PY'
import socket
import sys

port = int(sys.argv[1])
with socket.create_connection(("127.0.0.1", port), timeout=1):
    pass
PY
}

actual_mailpit_version() {
  "$mailpit_binary" version | awk '
    {
      for (field_index = 1; field_index <= NF; field_index++) {
        version = $field_index
        if (version ~ /^v?[0-9]+[.][0-9]+/) {
          sub(/^v/, "", version)
          print version
          exit
        }
      }
    }
  '
}

# Invoked by the EXIT trap below.
# shellcheck disable=SC2329
cleanup() {
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  if [ -n "$tmp_dir" ]; then
    rm -rf "$tmp_dir"
  fi
}

wait_for_stop() {
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ! kill -0 "$pid" 2>/dev/null; then
      wait "$pid" 2>/dev/null || true
      pid=
      return 0
    fi
    sleep 0.1
  done
  return 1
}

stop_mailpit() {
  if ! kill -0 "$pid" 2>/dev/null; then
    printf '%s\n' "Mailpit smoke failed: server exited before clean shutdown" >&2
    exit 44
  fi
  kill "$pid" 2>/dev/null || {
    printf '%s\n' "Mailpit smoke failed: could not request server shutdown" >&2
    exit 44
  }
  if ! wait_for_stop; then
    printf '%s\n' "Mailpit smoke failed: server did not stop cleanly" >&2
    exit 44
  fi
}

[ -x "$mailpit_binary" ] || {
  printf '%s\n' "missing executable bin/mailpit in $artifact_root" >&2
  exit 42
}
[ -n "$expected_version" ] || {
  printf '%s\n' "PV_UPSTREAM_VERSION is required for Mailpit smoke" >&2
  exit 42
}

need awk
need curl
need mktemp
need python3
need sleep

actual_version=$(actual_mailpit_version)
[ "$actual_version" = "$expected_version" ] || {
  printf '%s\n' "Mailpit version mismatch: expected $expected_version, got ${actual_version:-<unknown>}" >&2
  exit 43
}

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pv-mailpit-smoke.XXXXXX")
http_port=$(available_port)
smtp_port=$(available_port)
trap cleanup 0
"$mailpit_binary" \
  --database "$tmp_dir/mailpit.db" \
  --listen "127.0.0.1:$http_port" \
  --smtp "127.0.0.1:$smtp_port" \
  --disable-version-check \
  --quiet &
pid=$!

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl --fail --silent "http://127.0.0.1:$http_port/" >/dev/null &&
    check_tcp_bind "$smtp_port"; then
    stop_mailpit
    exit 0
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    printf '%s\n' "Mailpit HTTP UI or SMTP bind smoke failed: server exited early" >&2
    wait "$pid" 2>/dev/null || true
    pid=
    exit 43
  fi
  sleep 1
done

printf '%s\n' "Mailpit HTTP UI or SMTP bind smoke failed" >&2
exit 43
