#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

for version in 17.11 18.6; do
  image="postgres:$version-alpine"
  container="pv-postgres-import-fixtures-$version-$$"
  output="$root/$version"
  mkdir -p "$output"

  docker run --rm -d --name "$container" -e POSTGRES_PASSWORD=fixture-only "$image" >/dev/null
  trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT INT TERM
  attempt=0
  until docker exec "$container" pg_isready -U postgres >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
      docker logs "$container" >&2
      exit 1
    fi
    sleep 1
  done

  docker exec "$container" mkdir -p /tmp/pvfixture-tablespace
  docker exec "$container" chown postgres:postgres /tmp/pvfixture-tablespace
  docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres < "$root/source.sql" >/dev/null
  docker exec "$container" pg_dump -U postgres --restrict-key=pvfixture -d admin > "$output/plain.sql"
  docker exec "$container" pg_dump -U postgres --restrict-key=pvfixture --create -d admin > "$output/create.sql"
  docker exec "$container" pg_dump -U postgres --format=custom -d admin > "$output/custom.dump"
  docker exec "$container" pg_dump -U postgres --format=tar -d admin > "$output/tar.dump"
  docker exec "$container" pg_dump -U postgres --format=directory -f /tmp/admin.dir -d admin
  rm -rf "$output/directory"
  docker cp "$container:/tmp/admin.dir" "$output/directory" >/dev/null
  docker exec "$container" pg_dumpall -U postgres --no-role-passwords --restrict-key=pvfixture > "$output/all.sql"

  docker rm -f "$container" >/dev/null
  trap - EXIT INT TERM
done
