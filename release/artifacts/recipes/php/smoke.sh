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
  required_extensions=$1
  shift

  expected_extensions_sorted=$(printf '%s' "$required_extensions" | tr ',' '\n' | awk '
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

check_mbregex() {
  "$@" -r 'exit(function_exists("mb_split") ? 0 : 1);' || {
    printf '%s\n' "missing mbstring regex function: mb_split" >&2
    exit 47
  }
}

check_optional_extensions() {
  metadata="$artifact_root/share/pv/php-extensions.json"
  [ -f "$metadata" ] || return 0
  need python3
  scan_dir=$(mktemp -d)
  optional_extensions=$(python3 - "$metadata" "$artifact_root" "$scan_dir" <<'PY'
import json
import pathlib
import sys

metadata = pathlib.Path(sys.argv[1])
artifact_root = pathlib.Path(sys.argv[2])
scan_dir = pathlib.Path(sys.argv[3])
extension_names = []
for index, module in enumerate(json.loads(metadata.read_text())):
    extension_names.append(module["name"])
    directive = module["load_kind"]
    path = artifact_root / module["path"]
    prefix = 10 + index * 10
    (scan_dir / f"{prefix}-{module['name']}.ini").write_text(f"{directive}={path}\n")
print(",".join(extension_names))
PY
)
  if (
    PHP_INI_SCAN_DIR=$scan_dir
    export PHP_INI_SCAN_DIR
    check_extensions "$optional_extensions" "$@"
  ); then
    rm -rf "$scan_dir"
  else
    status=$?
    rm -rf "$scan_dir"
    return "$status"
  fi
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
  "$frankenphp_binary" php-cli -r 'printf("PHP %s\n", PHP_VERSION);' | grep -F "PHP $expected_version" >/dev/null
  check_extensions "$expected_extensions" "$frankenphp_binary" php-cli -r "foreach (get_loaded_extensions() as \$extension) { echo \$extension, PHP_EOL; }"
  check_mbregex "$frankenphp_binary" php-cli
  check_optional_extensions "$frankenphp_binary" php-cli -r "foreach (get_loaded_extensions() as \$extension) { echo \$extension, PHP_EOL; }"

  need python3
  site_dir=$(mktemp -d)
  cat >"$site_dir/index.php" <<'PHP'
<?php
echo "pv-frankenphp-ok\n";
phpinfo(INFO_CONFIGURATION);
PHP
  port=$(available_port)
  "$frankenphp_binary" php-server --listen "127.0.0.1:$port" --root "$site_dir" &
  pid=$!
  trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf "$site_dir"' 0
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    response=$(curl --fail --silent "http://127.0.0.1:$port/" || true)
    if printf '%s' "$response" | grep -F pv-frankenphp-ok >/dev/null; then
      if printf '%s' "$response" | grep -F '/usr/local/etc/php' >/dev/null; then
        printf '%s\n' "FrankenPHP artifact reports unsafe /usr/local/etc/php ini fallback" >&2
        exit 46
      fi
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
  check_extensions "$expected_extensions" "$php_binary" -m
  check_mbregex "$php_binary"
  check_optional_extensions "$php_binary" -m
  if "$php_binary" --ini 2>&1 | grep -F '/usr/local/etc/php' >/dev/null; then
    printf '%s\n' "PHP artifact reports unsafe /usr/local/etc/php ini fallback" >&2
    exit 46
  fi
  exit 0
fi

printf '%s\n' "artifact root has neither bin/php nor bin/frankenphp" >&2
exit 45
