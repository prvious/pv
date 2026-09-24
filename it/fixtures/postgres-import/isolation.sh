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

  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c "CREATE ROLE pv_import_fixture LOGIN PASSWORD 'fixture-only';"
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'CREATE DATABASE mapped_target;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'CREATE DATABASE second_target;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'CREATE DATABASE unrelated_project;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'GRANT CREATE ON DATABASE mapped_target TO pv_import_fixture;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'GRANT CREATE ON SCHEMA public TO pv_import_fixture;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d second_target -c 'GRANT CREATE ON SCHEMA public TO pv_import_fixture;'
  docker exec -u postgres "$container" sh -c 'printf "%s\n" "127.0.0.1:5432:mapped_target:pv_import_fixture:fixture-only" "127.0.0.1:5432:second_target:pv_import_fixture:fixture-only" > /tmp/pv-import-fixture.pass; chmod 600 /tmp/pv-import-fixture.pass'
  docker exec -u postgres "$container" sh -c 'cp "$PGDATA/pg_hba.conf" "$PGDATA/pg_hba.conf.before-import"'
  docker exec -u postgres "$container" sh -c \
    '{ printf "%s\n" "host mapped_target,second_target pv_import_fixture 127.0.0.1/32 scram-sha-256" "local all pv_import_fixture reject" "host all pv_import_fixture 0.0.0.0/0 reject" "host all pv_import_fixture ::/0 reject"; cat "$PGDATA/pg_hba.conf.before-import"; } > "$PGDATA/pg_hba.conf"'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'SELECT pg_reload_conf();'

  docker exec -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d mapped_target -c 'SELECT current_database();'
  docker exec -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d second_target -c 'SELECT current_database();'
  if docker exec -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d unrelated_project -c 'SELECT current_database();'; then
    echo "import role reached unrelated_project on Postgres $version" >&2
    exit 1
  fi
  if docker exec -u postgres "$container" psql -X -v ON_ERROR_STOP=1 -U pv_import_fixture -d unrelated_project -c 'SELECT current_database();'; then
    echo "import role reached unrelated_project through a local socket on Postgres $version" >&2
    exit 1
  fi
  if docker exec -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h ::1 -U pv_import_fixture -d unrelated_project -c 'SELECT current_database();'; then
    echo "import role reached unrelated_project through IPv6 on Postgres $version" >&2
    exit 1
  fi
  if printf '\\connect -reuse-previous=on unrelated_project\nSELECT current_database();\n' |
    docker exec -i -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d mapped_target; then
    echo "psql reused the import password to reach unrelated_project on Postgres $version" >&2
    exit 1
  fi
  if docker exec -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d mapped_target -c 'CREATE ROLE escaped;'; then
    echo "import role created another role on Postgres $version" >&2
    exit 1
  fi

  docker exec -i -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d mapped_target <<'SQL'
CREATE EXTENSION pgcrypto;
CREATE TABLE public.restored_proof (value integer);
CREATE FUNCTION public.double_value() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.value := NEW.value * 2;
  RETURN NEW;
END
$$;
CREATE TRIGGER double_value BEFORE INSERT ON public.restored_proof FOR EACH ROW EXECUTE FUNCTION public.double_value();
CREATE PUBLICATION proof_publication FOR TABLE public.restored_proof;
SELECT pg_catalog.lo_create(15000);
BEGIN;
SELECT pg_catalog.lo_open(15000, 131072);
SELECT pg_catalog.lowrite(0, '\x6869');
SELECT pg_catalog.lo_close(0);
COMMIT;
SQL
  docker exec -u postgres -e PGPASSFILE=/tmp/pv-import-fixture.pass "$container" psql -X -v ON_ERROR_STOP=1 -h 127.0.0.1 -U pv_import_fixture -d second_target -c 'CREATE TABLE public.second_proof (value integer); INSERT INTO public.second_proof VALUES (7);'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'REASSIGN OWNED BY pv_import_fixture TO postgres;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'DROP OWNED BY pv_import_fixture;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d second_target -c 'REASSIGN OWNED BY pv_import_fixture TO postgres;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d second_target -c 'DROP OWNED BY pv_import_fixture;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'DROP ROLE pv_import_fixture;'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c 'INSERT INTO public.restored_proof VALUES (21);'
  result=$(docker exec "$container" psql -X -A -t -v ON_ERROR_STOP=1 -U postgres -d mapped_target -c "SELECT (SELECT value FROM public.restored_proof) = 42 AND (SELECT count(*) FROM pg_extension WHERE extname = 'pgcrypto' AND extowner = (SELECT oid FROM pg_roles WHERE rolname = 'postgres')) = 1 AND (SELECT count(*) FROM pg_publication WHERE pubname = 'proof_publication' AND pubowner = (SELECT oid FROM pg_roles WHERE rolname = 'postgres')) = 1 AND (SELECT count(*) FROM pg_largeobject_metadata WHERE convert_from(pg_catalog.lo_get(oid), 'UTF8') = 'hi') = 1;")
  if [ "$result" != t ]; then
    echo "imported objects did not survive role cleanup on Postgres $version" >&2
    exit 1
  fi
  result=$(docker exec "$container" psql -X -A -t -v ON_ERROR_STOP=1 -U postgres -d second_target -c 'SELECT value = 7 FROM public.second_proof;')
  if [ "$result" != t ]; then
    echo "second target did not survive role cleanup on Postgres $version" >&2
    exit 1
  fi
  docker exec -u postgres "$container" sh -c 'cp "$PGDATA/pg_hba.conf.before-import" "$PGDATA/pg_hba.conf"; cmp "$PGDATA/pg_hba.conf.before-import" "$PGDATA/pg_hba.conf"'
  docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -c 'SELECT pg_reload_conf();'
  result=$(docker exec "$container" psql -X -A -t -v ON_ERROR_STOP=1 -U postgres -c "SELECT count(*) = 0 FROM pg_hba_file_rules WHERE 'pv_import_fixture' = ANY(user_name);")
  if [ "$result" != t ]; then
    echo "temporary HBA rules remain after cleanup on Postgres $version" >&2
    exit 1
  fi
  docker rm -f "$container" >/dev/null
  trap - EXIT INT TERM
done
