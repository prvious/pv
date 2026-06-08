#!/bin/sh
set -eu

artifact_root=$1
redis_server="$artifact_root/bin/redis-server"
redis_cli="$artifact_root/bin/redis-cli"
pid=

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

# Invoked by the EXIT trap below.
# shellcheck disable=SC2329
cleanup() {
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    "$redis_cli" -h 127.0.0.1 -p "$port" shutdown nosave >/dev/null 2>&1 || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$data_dir"
}

[ -x "$redis_server" ] || {
  printf '%s\n' "missing executable bin/redis-server in $artifact_root" >&2
  exit 42
}
[ -x "$redis_cli" ] || {
  printf '%s\n' "missing executable bin/redis-cli in $artifact_root" >&2
  exit 42
}

need mktemp
need python3

data_dir=$(mktemp -d "${TMPDIR:-/tmp}/pv-redis-smoke.XXXXXX")
port=$(available_port)
trap cleanup 0

"$redis_server" \
  --bind 127.0.0.1 \
  --port "$port" \
  --dir "$data_dir" \
  --save "" \
  --appendonly no \
  --daemonize no >/dev/null 2>&1 &
pid=$!

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if response=$("$redis_cli" -h 127.0.0.1 -p "$port" ping 2>/dev/null) &&
    [ "$response" = "PONG" ]; then
    "$redis_cli" -h 127.0.0.1 -p "$port" shutdown nosave >/dev/null
    wait "$pid"
    pid=
    exit 0
  fi
  sleep 1
done

printf '%s\n' "Redis smoke failed: redis-cli ping did not return PONG" >&2
exit 43
