#!/bin/sh
set -eu

data_dir=""
username=""
password_file=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -D)
      data_dir="$2"
      shift 2
      ;;
    -U)
      username="$2"
      shift 2
      ;;
    --username)
      username="$2"
      shift 2
      ;;
    --pwfile)
      password_file="$2"
      shift 2
      ;;
    --auth-host|--auth-local)
      shift 2
      ;;
    *)
      echo "unexpected initdb argument: $1" >&2
      exit 64
      ;;
  esac
done

if [ -z "$data_dir" ] || [ -z "$username" ] || [ -z "$password_file" ]; then
  echo "missing initdb inputs" >&2
  exit 64
fi

if [ -d "$data_dir" ] && [ "$(find "$data_dir" -mindepth 1 -maxdepth 1 | wc -l)" -gt 0 ]; then
  echo "PGDATA is not empty before initdb" >&2
  exit 65
fi

mkdir -p "$data_dir/databases"
printf '16\n' > "$data_dir/PG_VERSION"
printf '%s\n' "$username" > "$data_dir/initdb.username"
cat "$password_file" > "$data_dir/initdb.password"
