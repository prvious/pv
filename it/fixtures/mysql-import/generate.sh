#!/bin/sh
set -eu

fixture_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

dump() {
  docker exec "$container" mysqldump -uroot -pfixture-only \
    --default-character-set=utf8mb4 --hex-blob --routines --events --triggers \
    --set-gtid-purged=OFF --column-statistics=0 --skip-dump-date "$@"
}

for version in 8.0.46 8.4.9 9.7.0; do
  container="pv350-fixture-${version}-$$"
  output_dir="$fixture_dir/$version"
  mkdir -p "$output_dir"
  docker run --rm -d --name "$container" -e MYSQL_ROOT_PASSWORD=fixture-only "mysql:$version" >/dev/null
  trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT INT TERM

  attempt=0
  until docker exec "$container" mysql -uroot -pfixture-only -N -e 'SELECT 1' >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 60 ]; then
      docker logs "$container" >&2
      exit 1
    fi
    sleep 1
  done

  docker exec -i "$container" mysql -uroot -pfixture-only \
    --default-character-set=utf8mb4 --batch < "$fixture_dir/source.sql"
  dump admin > "$output_dir/plain.sql"
  dump --databases admin analytics 'Mixed-Name' > "$output_dir/databases.sql"
  dump --add-drop-database --databases admin analytics 'Mixed-Name' > "$output_dir/drop-databases.sql"
  dump --no-create-db --databases admin analytics 'Mixed-Name' > "$output_dir/no-create.sql"
  dump --all-databases --no-data > "$output_dir/all.sql"

  docker rm -f "$container" >/dev/null
  trap - EXIT INT TERM
done
