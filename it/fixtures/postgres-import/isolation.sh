#!/bin/sh
set -eu

for version in 17.11 18.6; do
  container="pv-postgres-import-isolation-$version-$$"
  docker run --rm -d --name "$container" -e POSTGRES_PASSWORD=fixture-only "postgres:$version-alpine" >/dev/null
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

  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'CREATE ROLE pv_import_fixture LOGIN;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'CREATE DATABASE mapped_target;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'CREATE DATABASE unrelated_project;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'GRANT CREATE ON SCHEMA public TO pv_import_fixture;'
  docker exec -u postgres "$container" sh -c 'cp "$PGDATA/pg_hba.conf" "$PGDATA/pg_hba.conf.before-import"'
  docker exec -u postgres "$container" sh -c \
    '{ printf "%s\n" "host mapped_target pv_import_fixture 127.0.0.1/32 trust" "host all pv_import_fixture 127.0.0.1/32 reject"; cat "$PGDATA/pg_hba.conf.before-import"; } > "$PGDATA/pg_hba.conf"'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'SELECT pg_reload_conf();'

  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d mapped_target -c 'SELECT current_database();'
  if docker exec "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d unrelated_project -c 'SELECT current_database();'; then
    echo "import role reached unrelated_project on Postgres $version" >&2
    exit 1
  fi
  if docker exec "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d mapped_target -c 'CREATE ROLE escaped;'; then
    echo "import role created another role on Postgres $version" >&2
    exit 1
  fi

  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d mapped_target -c 'CREATE TABLE public.restored_proof (value integer); INSERT INTO public.restored_proof VALUES (42);'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'REASSIGN OWNED BY pv_import_fixture TO postgres;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'DROP OWNED BY pv_import_fixture;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'DROP ROLE pv_import_fixture;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'SELECT value FROM public.restored_proof;'
  docker exec -u postgres "$container" sh -c 'mv "$PGDATA/pg_hba.conf.before-import" "$PGDATA/pg_hba.conf"'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'SELECT pg_reload_conf();'
  docker rm -f "$container" >/dev/null
  trap - EXIT INT TERM
done
