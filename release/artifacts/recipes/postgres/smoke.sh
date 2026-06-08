#!/bin/sh
set -eu

artifact_root=$1
postgres="$artifact_root/bin/postgres"
initdb="$artifact_root/bin/initdb"
pg_ctl="$artifact_root/bin/pg_ctl"
psql="$artifact_root/bin/psql"

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

[ -x "$postgres" ] || {
  printf '%s\n' "missing executable $postgres" >&2
  exit 42
}
[ -x "$initdb" ] || {
  printf '%s\n' "missing executable $initdb" >&2
  exit 42
}
[ -x "$pg_ctl" ] || {
  printf '%s\n' "missing executable $pg_ctl" >&2
  exit 42
}
[ -x "$psql" ] || {
  printf '%s\n' "missing executable $psql" >&2
  exit 42
}

need id
need mktemp
need python3

tmpdir=$(mktemp -d)
datadir="$tmpdir/data"
socket_dir="$tmpdir/socket"
log_file="$tmpdir/postgres.log"
port=$(available_port)
user=$(id -un)
started=false

trap 'if [ "$started" = true ]; then "$pg_ctl" -D "$datadir" -m fast stop >/dev/null 2>&1 || true; fi; rm -rf "$tmpdir"' 0 1 2 3 15

mkdir -p "$socket_dir"
"$initdb" -D "$datadir" --username="$user" --no-locale --encoding=UTF8 >/dev/null
"$pg_ctl" -D "$datadir" -l "$log_file" -o "-h 127.0.0.1 -p $port -k $socket_dir" start >/dev/null
started=true

for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  if "$psql" -h 127.0.0.1 -p "$port" -U "$user" -d postgres -At -c 'SELECT 1' >"$tmpdir/select.out" 2>"$tmpdir/select.err"; then
    select_output=$(cat "$tmpdir/select.out")
    [ "$select_output" = "1" ] || {
      printf '%s\n' "Postgres SELECT 1 smoke returned: $select_output" >&2
      exit 43
    }
    "$pg_ctl" -D "$datadir" -m fast stop >/dev/null
    started=false
    exit 0
  fi
  sleep 1
done

printf '%s\n' "Postgres smoke failed to become ready; log follows:" >&2
if [ -f "$log_file" ]; then
  cat "$log_file" >&2
fi
if [ -f "$tmpdir/select.err" ]; then
  cat "$tmpdir/select.err" >&2
fi
exit 44
