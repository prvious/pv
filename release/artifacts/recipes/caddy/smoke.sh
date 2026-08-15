#!/bin/sh
set -eu

artifact_root=$1
caddy_binary="$artifact_root/bin/caddy"
expected_version=${PV_UPSTREAM_VERSION:-2.11.4}
pid=
backend_pid=
tmp_dir=

die() {
  printf '%s\n' "Caddy smoke failed: $*" >&2
  exit 43
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

available_port() {
  python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

actual_caddy_version() {
  "$caddy_binary" version | awk '
    {
      for (field_index = 1; field_index <= NF; field_index++) {
        version = $field_index
        if (version ~ /^v?[0-9]+[.][0-9]+[.][0-9]+$/) {
          sub(/^v/, "", version)
          print version
          exit
        }
      }
    }
  '
}

print_caddy_log() {
  if [ -n "$caddy_log" ] && [ -s "$caddy_log" ]; then
    printf '%s\n' "Caddy output:" >&2
    sed 's/^/  /' "$caddy_log" >&2
  fi
}

stop_process() {
  process_pid=$1
  process_name=$2
  if ! kill -0 "$process_pid" 2>/dev/null; then
    wait "$process_pid" 2>/dev/null || true
    return 0
  fi

  if kill -TERM "$process_pid" 2>/dev/null; then
    for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
      if ! kill -0 "$process_pid" 2>/dev/null; then
        wait "$process_pid" 2>/dev/null || true
        return 0
      fi
      sleep 0.1
    done
  elif ! kill -0 "$process_pid" 2>/dev/null; then
    wait "$process_pid" 2>/dev/null || true
    return 0
  fi

  printf '%s\n' "Caddy smoke: $process_name did not stop after TERM; sending KILL" >&2
  kill -KILL "$process_pid" 2>/dev/null || true
  wait "$process_pid" 2>/dev/null || true
  if kill -0 "$process_pid" 2>/dev/null; then
    print_caddy_log
    printf '%s\n' "Caddy smoke failed: $process_name could not be reaped" >&2
    return 1
  fi
  return 0
}

# Invoked by the EXIT trap below.
# shellcheck disable=SC2329
cleanup() {
  exit_status=$?
  cleanup_status=0
  if [ -n "$pid" ]; then
    if ! stop_process "$pid" caddy; then
      cleanup_status=1
    fi
    pid=
  fi
  if [ -n "$backend_pid" ]; then
    if ! stop_process "$backend_pid" backend; then
      cleanup_status=1
    fi
    backend_pid=
  fi
  if [ -n "$tmp_dir" ]; then
    if ! rm -rf "$tmp_dir"; then
      cleanup_status=1
    fi
    tmp_dir=
  fi
  if [ "$cleanup_status" -ne 0 ]; then
    printf '%s\n' "Caddy smoke failed: cleanup did not complete reliably" >&2
    [ "$exit_status" -ne 0 ] || exit_status=44
  fi
  exit "$exit_status"
}

write_config() {
  config_path=$1
  health_response=$2
  cat >"$config_path" <<EOF
{
    admin 127.0.0.1:$admin_port
    persist_config off
    storage file_system {
        root $storage_dir
    }
    local_certs
}

http://127.0.0.1:$http_port {
    @health path /health
    respond @health "$health_response" 200
    handle /proxy/* {
        reverse_proxy 127.0.0.1:$backend_port
    }
}

https://127.0.0.1:$https_port {
    tls internal
    @health path /health
    respond @health "$health_response" 200
}
EOF
}

[ -x "$caddy_binary" ] || die "missing executable bin/caddy in $artifact_root"
[ "$expected_version" = "2.11.4" ] || die "expected Caddy smoke version 2.11.4, got $expected_version"

need awk
need curl
need mktemp
need python3
need sed
need sleep

actual_version=$(actual_caddy_version)
[ "$actual_version" = "$expected_version" ] || die "version mismatch: expected $expected_version, got ${actual_version:-<unknown>}"

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/pv-caddy-smoke.XXXXXX")
storage_dir="$tmp_dir/storage"
config_path="$tmp_dir/Caddyfile"
changed_config_path="$tmp_dir/Caddyfile.changed"
invalid_config_path="$tmp_dir/Caddyfile.invalid"
caddy_log="$tmp_dir/caddy.log"
backend_port=$(available_port)
http_port=$(available_port)
https_port=$(available_port)
admin_port=$(available_port)
trap cleanup 0
initial_checks_passed=0

mkdir -p "$storage_dir"
python3 -u - "$backend_port" <<'PY' &
import http.server
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"backend-v1"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler)
server.serve_forever()
PY
backend_pid=$!

write_config "$config_path" health-v1
"$caddy_binary" validate --config "$config_path" --adapter caddyfile
"$caddy_binary" run --config "$config_path" --adapter caddyfile >"$caddy_log" 2>&1 &
pid=$!

for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30; do
  if curl --fail --silent "http://127.0.0.1:$admin_port/config/" >/dev/null &&
    [ "$(curl --fail --silent "http://127.0.0.1:$http_port/health")" = "health-v1" ] &&
    [ "$(curl --fail --silent "http://127.0.0.1:$http_port/proxy/check")" = "backend-v1" ] &&
    [ "$(curl --fail --silent --insecure "https://127.0.0.1:$https_port/health")" = "health-v1" ]; then
    initial_checks_passed=1
    break
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    print_caddy_log
    die "Caddy exited before initial health, proxy, and TLS checks passed"
  fi
  sleep 0.2
done

if [ "$initial_checks_passed" -ne 1 ]; then
  print_caddy_log
  die "initial admin, health, proxy, and HTTPS/TLS checks did not become ready"
fi

[ "$(curl --fail --silent "http://127.0.0.1:$http_port/health")" = "health-v1" ] || {
  print_caddy_log
  die "initial health response was not active"
}
[ "$(curl --fail --silent "http://127.0.0.1:$http_port/proxy/check")" = "backend-v1" ] || {
  print_caddy_log
  die "initial reverse proxy response was not active"
}

write_config "$changed_config_path" health-v2
"$caddy_binary" validate --config "$changed_config_path" --adapter caddyfile
curl --fail --silent --show-error \
  -X POST \
  -H 'Content-Type: text/caddyfile' \
  --data-binary "@$changed_config_path" \
  "http://127.0.0.1:$admin_port/load" >/dev/null

[ "$(curl --fail --silent "http://127.0.0.1:$http_port/health")" = "health-v2" ] || {
  print_caddy_log
  die "POST /load did not activate the changed health response"
}
kill -0 "$pid" 2>/dev/null || die "Caddy PID changed during POST /load"

cat >"$invalid_config_path" <<EOF
{
    admin 127.0.0.1:$admin_port
    persist_config off
    definitely_not_a_caddyfile_directive
}
EOF
invalid_status=$(curl --silent --show-error \
  -o "$tmp_dir/invalid-response" \
  -w '%{http_code}' \
  -X POST \
  -H 'Content-Type: text/caddyfile' \
  --data-binary "@$invalid_config_path" \
  "http://127.0.0.1:$admin_port/load")
case "$invalid_status" in
  4* | 5*) ;;
  *) print_caddy_log; die "invalid POST /load returned HTTP $invalid_status" ;;
esac

[ "$(curl --fail --silent "http://127.0.0.1:$http_port/health")" = "health-v2" ] || {
  print_caddy_log
  die "invalid POST /load replaced the previously active response"
}

stop_process "$pid" caddy
pid=
stop_process "$backend_pid" backend
backend_pid=
exit 0
