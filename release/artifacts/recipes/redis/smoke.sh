#!/bin/sh
set -eu

artifact_root=$1
redis_server="$artifact_root/bin/redis-server"
redis_cli="$artifact_root/bin/redis-cli"
pid=
log_file=

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

print_server_log() {
  if [ -n "$log_file" ] && [ -s "$log_file" ]; then
    printf '%s\n' "redis-server output:" >&2
    sed 's/^/  /' "$log_file" >&2
  fi
}

# Invoked by the EXIT trap below.
# shellcheck disable=SC2329
cleanup() {
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    if ! "$redis_cli" -h 127.0.0.1 -p "$port" shutdown nosave >/dev/null 2>&1; then
      kill "$pid" 2>/dev/null || true
      sleep 0.1
      if kill -0 "$pid" 2>/dev/null; then
        kill -KILL "$pid" 2>/dev/null || true
      fi
    fi
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
need sed
need sleep

data_dir=$(mktemp -d "${TMPDIR:-/tmp}/pv-redis-smoke.XXXXXX")
log_file="$data_dir/redis-server.log"
port=$(available_port)
trap cleanup 0

"$redis_server" \
  --bind 127.0.0.1 \
  --port "$port" \
  --dir "$data_dir" \
  --save "" \
  --appendonly no \
  --daemonize no >"$log_file" 2>&1 &
pid=$!

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if response=$("$redis_cli" -h 127.0.0.1 -p "$port" ping 2>/dev/null) &&
    [ "$response" = "PONG" ]; then
    "$redis_cli" -h 127.0.0.1 -p "$port" shutdown nosave >/dev/null
    wait "$pid"
    pid=
    exit 0
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    printf '%s\n' "Redis smoke failed: redis-server exited before accepting connections" >&2
    print_server_log
    wait "$pid" 2>/dev/null || true
    pid=
    exit 43
  fi
  sleep 1
done

printf '%s\n' "Redis smoke failed: redis-cli ping did not return PONG" >&2
print_server_log
exit 43
