#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s up <name> [image] | exec <name> <command...> | down <name>\n' "$0" >&2
  exit 2
}

[[ $# -ge 2 ]] || usage
action=$1
name=$2
shift 2
[[ "$name" =~ ^[a-z0-9][a-z0-9_.-]*$ ]] || usage
worktree=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
container="pv-linux-$name"
owner_label=dev.pv.linux-worktree

require_owned_container() {
  local owner
  owner=$(docker inspect --format "{{ index .Config.Labels \"$owner_label\" }}" "$container")
  if [[ "$owner" != "$worktree" ]]; then
    printf 'Refusing to operate on %s: it belongs to another worktree.\n' "$container" >&2
    exit 1
  fi
}

case "$action" in
  up)
    [[ $# -le 1 ]] || usage
    image=${1:-pv-linux-test:$name}
    docker run --detach --name "$container" --label "$owner_label=$worktree" \
      --privileged --cgroupns=private --env container=docker \
      --tmpfs /run --tmpfs /run/lock --tmpfs /tmp:exec \
      --mount "type=bind,src=$worktree,dst=/src,readonly" "$image" /sbin/init
    for attempt in {1..30}; do
      if docker exec "$container" test -S /run/dbus/system_bus_socket; then
        break
      fi
      if [[ $attempt -eq 30 ]]; then
        printf 'System bus did not become ready in %s. Inspect docker logs before cleanup.\n' "$container" >&2
        exit 1
      fi
      sleep 1
    done
    docker exec "$container" loginctl enable-linger dev
    docker exec "$container" systemctl start user@1000.service
    docker exec "$container" sh -c 'printf "nameserver 127.0.0.53\n" > /etc/resolv.conf'
    ;;
  exec)
    [[ $# -gt 0 ]] || usage
    require_owned_container
    docker exec --user dev --env XDG_RUNTIME_DIR=/run/user/1000 \
      --env DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "$container" "$@"
    ;;
  down)
    [[ $# -eq 0 ]] || usage
    require_owned_container
    docker stop --time 20 "$container"
    docker rm "$container"
    ;;
  *) usage ;;
esac
