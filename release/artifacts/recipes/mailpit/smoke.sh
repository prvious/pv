#!/bin/sh
set -eu

artifact_root=$1
mailpit_binary="$artifact_root/bin/mailpit"
expected_version=${PV_UPSTREAM_VERSION:-}

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

[ -x "$mailpit_binary" ] || {
  printf '%s\n' "missing executable bin/mailpit in $artifact_root" >&2
  exit 42
}
[ -n "$expected_version" ] || {
  printf '%s\n' "PV_UPSTREAM_VERSION is required for Mailpit smoke" >&2
  exit 42
}

need curl
need grep
need mktemp
need python3

"$mailpit_binary" version | grep -F "v$expected_version" >/dev/null

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pv-mailpit-smoke.XXXXXX")
http_port=$(available_port)
smtp_port=$(available_port)
"$mailpit_binary" \
  --database "$tmp_dir/mailpit.db" \
  --listen "127.0.0.1:$http_port" \
  --smtp "127.0.0.1:$smtp_port" \
  --disable-version-check \
  --quiet &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf "$tmp_dir"' 0

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl --fail --silent "http://127.0.0.1:$http_port/" >/dev/null &&
    check_tcp_bind "$smtp_port"; then
    exit 0
  fi
  sleep 1
done

printf '%s\n' "Mailpit HTTP UI or SMTP bind smoke failed" >&2
exit 43
