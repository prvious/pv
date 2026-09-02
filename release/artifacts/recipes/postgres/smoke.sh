#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../../.." && pwd)
# shellcheck source=/dev/null
. "$ROOT/release/artifacts/recipes/common.sh"

artifact_root=$1
postgres="$artifact_root/bin/postgres"
initdb="$artifact_root/bin/initdb"
pg_ctl="$artifact_root/bin/pg_ctl"
psql="$artifact_root/bin/psql"
extension_catalog=${PV_POSTGRES_EXTENSION_CATALOG:-}
admin_user=pv_root
admin_password=pv_local_password
platform=${PV_POSTGRES_PLATFORM:-}
deployment_target=${PV_POSTGRES_DEPLOYMENT_TARGET:-}

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

run_psql() (
  database=$1
  sql=$2
  PGPASSWORD="$admin_password" "$psql" \
    -X \
    -h 127.0.0.1 \
    -p "$port" \
    -U "$admin_user" \
    -d "$database" \
    -v ON_ERROR_STOP=1 \
    -qAt \
    -c "$sql"
)

wait_until_ready() {
  for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
    if run_psql postgres 'SELECT 1' >"$tmpdir/select.out" 2>"$tmpdir/select.err"; then
      select_output=$(cat "$tmpdir/select.out")
      [ "$select_output" = "1" ] || {
        printf '%s\n' "Postgres SELECT 1 smoke returned: $select_output" >&2
        exit 43
      }
      return
    fi
    sleep 1
  done

  printf '%s\n' "Postgres smoke failed to become ready; log follows:" >&2
  [ ! -f "$log_file" ] || cat "$log_file" >&2
  [ ! -f "$tmpdir/select.err" ] || cat "$tmpdir/select.err" >&2
  exit 44
}

assert_query_true() {
  database=$1
  sql=$2
  description=$3
  output=$(run_psql "$database" "$sql")
  [ "$output" = "t" ] || {
    printf '%s\n' "$description returned: $output" >&2
    exit 45
  }
}

validate_packaged_macho() {
  macho=$1
  pv_recipe_validate_macho_binary "$macho" "$platform" "$deployment_target"
  codesign --verify "$macho"
}

validate_packaged_macho_tree() {
  for relative_path in \
    bin/postgres \
    bin/initdb \
    bin/pg_ctl \
    bin/psql \
    lib/libcrypto.3.dylib \
    lib/libssl.3.dylib \
    lib/postgresql/pg_trgm.dylib \
    lib/postgresql/pgcrypto.dylib \
    lib/postgresql/sslinfo.dylib; do
    [ -f "$artifact_root/$relative_path" ] || die "Postgres archive missing $relative_path"
    validate_packaged_macho "$artifact_root/$relative_path"
  done

  for macho_dir in "$artifact_root/bin" "$artifact_root/lib"; do
    find "$macho_dir" -type f | while IFS= read -r macho; do
      pv_recipe_is_macho "$macho" || continue
      validate_packaged_macho "$macho"
    done
  done
}

# Invoked by the EXIT trap below.
# shellcheck disable=SC2329
cleanup() {
  status=$?
  trap - 0 1 2 3 15

  if [ "$started" = true ] && ! "$pg_ctl" -D "$datadir" -m fast stop >/dev/null; then
    printf '%s\n' "Postgres smoke cleanup failed to stop the server" >&2
    [ "$status" -ne 0 ] || status=46
  fi

  if [ "$status" -ne 0 ]; then
    [ ! -f "$log_file" ] || cat "$log_file" >&2
    printf '%s\n' "Postgres smoke files retained at $tmpdir" >&2
  else
    rm -rf "$tmpdir"
  fi
  exit "$status"
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

need diff
need codesign
need file
need find
need lipo
need mktemp
need otool
need python3
[ -n "$platform" ] || die "missing PV_POSTGRES_PLATFORM for packaged Mach-O validation"
[ -n "$deployment_target" ] || die "missing PV_POSTGRES_DEPLOYMENT_TARGET for packaged Mach-O validation"
[ -f "$extension_catalog" ] || {
  printf '%s\n' "missing Postgres extension catalog: $extension_catalog" >&2
  exit 42
}

tmpdir=$(mktemp -d)
datadir="$tmpdir/data"
log_file="$tmpdir/postgres.log"
password_file="$tmpdir/initdb-password"
actual_available="$tmpdir/available-extensions.txt"
actual_installed="$tmpdir/installed-extensions.txt"
port=$(available_port)
started=false

trap cleanup 0 1 2 3 15

validate_packaged_macho_tree

printf '%s\n' "$admin_password" >"$password_file"
"$initdb" \
  -D "$datadir" \
  --username "$admin_user" \
  --pwfile "$password_file" \
  --auth-host scram-sha-256 \
  --auth-local trust \
  --no-locale \
  --encoding UTF8 >/dev/null
rm "$password_file"
printf '%s\n' \
  "listen_addresses = '127.0.0.1'" \
  "port = $port" \
  "unix_socket_directories = ''" >"$datadir/postgresql.conf"
"$pg_ctl" -D "$datadir" -l "$log_file" start >/dev/null
started=true
wait_until_ready

run_psql postgres 'COPY (SELECT name FROM pg_available_extensions ORDER BY name) TO STDOUT' >"$actual_available"
diff -u "$extension_catalog" "$actual_available"
run_psql postgres 'COPY (SELECT extname FROM pg_extension ORDER BY extname) TO STDOUT' >"$actual_installed"
printf '%s\n' plpgsql >"$tmpdir/expected-installed-extensions.txt"
diff -u "$tmpdir/expected-installed-extensions.txt" "$actual_installed"

while IFS= read -r extension; do
  [ "$extension" != plpgsql ] || continue
  database="smoke_$extension"
  run_psql postgres "CREATE DATABASE \"$database\"" >/dev/null
  if [ "$extension" = earthdistance ]; then
    run_psql "$database" 'CREATE EXTENSION earthdistance CASCADE' >/dev/null
  else
    run_psql "$database" "CREATE EXTENSION \"$extension\"" >/dev/null
  fi

  case "$extension" in
    pg_trgm)
      run_psql "$database" "CREATE TABLE trgm_smoke(value text); INSERT INTO trgm_smoke VALUES ('PostgreSQL'); CREATE INDEX trgm_smoke_idx ON trgm_smoke USING gin (value gin_trgm_ops); SET enable_seqscan = off" >/dev/null
      assert_query_true "$database" "SELECT count(*) = 1 FROM trgm_smoke WHERE value % 'postgres'" "pg_trgm indexed similarity smoke"
      ;;
    pgcrypto)
      assert_query_true "$database" "SELECT octet_length(digest('pv', 'sha256')) = 32 AND left(gen_salt('md5'), 3) = '\$1\$' AND pgp_sym_decrypt(pgp_sym_encrypt('pv', 'secret', 'cipher-algo=aes256'), 'secret') = 'pv'" "pgcrypto digest, gen_salt, and AES smoke"
      ;;
    sslinfo)
      assert_query_true "$database" 'SELECT NOT ssl_is_used()' "sslinfo smoke"
      ;;
  esac

  run_psql postgres "DROP DATABASE \"$database\"" >/dev/null
done <"$extension_catalog"

"$pg_ctl" -D "$datadir" -m fast stop >/dev/null
started=false
"$pg_ctl" -D "$datadir" -l "$log_file" start >/dev/null
started=true
wait_until_ready
run_psql postgres 'COPY (SELECT name FROM pg_available_extensions ORDER BY name) TO STDOUT' >"$actual_available"
diff -u "$extension_catalog" "$actual_available"

"$pg_ctl" -D "$datadir" -m fast stop >/dev/null
started=false
exit 0
