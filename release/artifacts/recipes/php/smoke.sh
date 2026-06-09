#!/bin/sh
set -eu

artifact_root=$1
expected_extensions=${PV_EXPECTED_EXTENSIONS:-}
upstream_version=${PV_UPSTREAM_VERSION:-}

need() {
  command -v "$1" >/dev/null 2>&1 || {
    printf '%s\n' "missing required command: $1" >&2
    exit 42
  }
}

check_extensions() {
  expected_extensions_sorted=$(printf '%s' "$expected_extensions" | tr ',' '\n' | awk '
    {
      sub(/^[[:space:]]+/, "")
      sub(/[[:space:]]+$/, "")
      if ($0 != "") {
        print tolower($0)
      }
    }
  ' | sort -u | tr '\n' ',')
  actual_extensions=$("$@" | awk '
    BEGIN {
      ignored_runtime_modules = "^(core|date|random|reflection|spl|standard|mysqlnd)$"
    }
    {
      sub(/^[[:space:]]+/, "")
      sub(/[[:space:]]+$/, "")
      extension = tolower($0)
      if (extension == "" || extension ~ /^\[[^]]+\]$/) {
        next
      }
      if (extension ~ ignored_runtime_modules) {
        next
      }
      print extension
    }
  ' | sort -u | tr '\n' ',')
  old_ifs=$IFS
  IFS=,
  for extension in $expected_extensions_sorted; do
    [ -n "$extension" ] || continue
    case ",$actual_extensions" in
      *,"$extension",*) ;;
      *)
        printf '%s\n' "missing PHP extension: $extension" >&2
        exit 43
        ;;
    esac
  done
  IFS=$old_ifs
}

available_port() {
  python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

[ -n "$expected_extensions" ] || {
  printf '%s\n' "PV_EXPECTED_EXTENSIONS is required for PHP smoke" >&2
  exit 42
}
[ -n "$upstream_version" ] || {
  printf '%s\n' "PV_UPSTREAM_VERSION is required for PHP smoke" >&2
  exit 42
}

need awk
need curl
need grep
need mktemp
need sort
need tr

expected_version=${upstream_version%%-frankenphp*}

if [ -x "$artifact_root/bin/frankenphp" ]; then
  frankenphp_binary="$artifact_root/bin/frankenphp"
  "$frankenphp_binary" php-cli -v | grep -F "PHP $expected_version" >/dev/null
  check_extensions "$frankenphp_binary" php-cli -m

  need python3
  site_dir=$(mktemp -d)
  cat >"$site_dir/index.php" <<'PHP'
<?php echo "pv-frankenphp-ok";
PHP
  port=$(available_port)
  "$frankenphp_binary" php-server --listen "127.0.0.1:$port" --root "$site_dir" &
  pid=$!
  trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf "$site_dir"' 0
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl --fail --silent "http://127.0.0.1:$port/" | grep -F pv-frankenphp-ok >/dev/null; then
      exit 0
    fi
    sleep 1
  done
  printf '%s\n' "FrankenPHP loopback smoke failed" >&2
  exit 44
fi

if [ -x "$artifact_root/bin/php" ]; then
  php_binary="$artifact_root/bin/php"
  "$php_binary" -v | grep -F "PHP $expected_version" >/dev/null
  check_extensions "$php_binary" -m
  exit 0
fi

printf '%s\n' "artifact root has neither bin/php nor bin/frankenphp" >&2
exit 45
