#!/bin/sh
set -eu

artifact_root=$1
mysqld="$artifact_root/bin/mysqld"
mysql="$artifact_root/bin/mysql"
mysqladmin="$artifact_root/bin/mysqladmin"

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

[ -x "$mysqld" ] || {
  printf '%s\n' "missing executable $mysqld" >&2
  exit 42
}
[ -x "$mysql" ] || {
  printf '%s\n' "missing executable $mysql" >&2
  exit 42
}
[ -x "$mysqladmin" ] || {
  printf '%s\n' "missing executable $mysqladmin" >&2
  exit 42
}

need grep
need mktemp
need python3

tmpdir=$(mktemp -d)
datadir="$tmpdir/data"
socket_path="$tmpdir/mysql.sock"
pid_file="$tmpdir/mysqld.pid"
port=$(available_port)
server_pid=

trap 'if [ -n "$server_pid" ]; then "$mysqladmin" --protocol=tcp --host=127.0.0.1 --port="$port" --user=root shutdown >/dev/null 2>&1 || true; wait "$server_pid" 2>/dev/null || true; fi; rm -rf "$tmpdir"' 0 1 2 3 15

"$mysqld" \
  --no-defaults \
  --initialize-insecure \
  --basedir="$artifact_root" \
  --datadir="$datadir" \
  --log-error="$tmpdir/init.err"

"$mysqld" \
  --no-defaults \
  --basedir="$artifact_root" \
  --datadir="$datadir" \
  --socket="$socket_path" \
  --port="$port" \
  --bind-address=127.0.0.1 \
  --pid-file="$pid_file" \
  --mysqlx=OFF \
  --log-error="$tmpdir/mysqld.err" &
server_pid=$!

for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  if "$mysqladmin" --protocol=tcp --host=127.0.0.1 --port="$port" --user=root ping >/dev/null 2>&1; then
    select_output=$("$mysql" --protocol=tcp --host=127.0.0.1 --port="$port" --user=root --batch --skip-column-names -e 'SELECT 1')
    printf '%s\n' "$select_output" | grep -Fx 1 >/dev/null || {
      printf '%s\n' "MySQL SELECT 1 smoke returned: $select_output" >&2
      exit 43
    }
    "$mysqladmin" --protocol=tcp --host=127.0.0.1 --port="$port" --user=root shutdown
    wait "$server_pid"
    server_pid=
    exit 0
  fi
  sleep 1
done

printf '%s\n' "MySQL smoke failed to become ready; log follows:" >&2
if [ -f "$tmpdir/mysqld.err" ]; then
  cat "$tmpdir/mysqld.err" >&2
fi
exit 44
