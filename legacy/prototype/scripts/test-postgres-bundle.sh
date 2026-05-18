#!/bin/bash
# test-postgres-bundle.sh — integration test for a built PostgreSQL bundle.
#
# Verifies that an extracted theseus-rs PostgreSQL bundle (after our patching
# pass) actually works end-to-end: initdb, server startup, every bundled
# contrib extension loads, common extensions exercise correctly, pgbench runs
# a real workload, pg_amcheck reports no corruption, and pg_dump → restore
# round-trips.
#
# Usage:
#     scripts/test-postgres-bundle.sh <path-to-extracted-bundle>
#
# The path argument should be the directory containing bin/, lib/, share/,
# and include/. Used by .github/workflows/build-artifacts.yml against the
# patched staging tree, and locally by passing an extracted CI artifact.
#
# Exit code: 0 = all checks passed; non-zero = at least one failed.

set -euo pipefail

PG_PREFIX="${1:?usage: $0 <path-to-extracted-bundle>}"
PORT="${POSTGRES_TEST_PORT:-54199}"

if [ ! -x "$PG_PREFIX/bin/postgres" ]; then
    echo "::error::No bin/postgres at $PG_PREFIX"
    exit 1
fi

DATA_DIR="$(mktemp -d)/pgdata"
PG_LOG="$(mktemp)"
PG_PID=""

cleanup() {
    if [ -n "$PG_PID" ]; then
        kill "$PG_PID" 2>/dev/null || true
        wait "$PG_PID" 2>/dev/null || true
    fi
    rm -rf "$(dirname "$DATA_DIR")"
    rm -f "$PG_LOG"
}
trap cleanup EXIT

PSQL=("$PG_PREFIX/bin/psql" -h 127.0.0.1 -p "$PORT" -U postgres -X -q)

# 1. initdb
echo "== 1. initdb =="
"$PG_PREFIX/bin/initdb" -D "$DATA_DIR" -U postgres --auth=trust >/dev/null
echo "  ✓ cluster initialized at $DATA_DIR"

# 2. start postgres
echo "== 2. start postgres =="
"$PG_PREFIX/bin/postgres" -D "$DATA_DIR" -p "$PORT" -k "$DATA_DIR" >"$PG_LOG" 2>&1 &
PG_PID=$!

for i in $(seq 1 15); do
    if "$PG_PREFIX/bin/pg_isready" -h 127.0.0.1 -p "$PORT" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

if ! "$PG_PREFIX/bin/pg_isready" -h 127.0.0.1 -p "$PORT" >/dev/null 2>&1; then
    echo "::error::postgres did not become ready within 15s"
    cat "$PG_LOG"
    exit 1
fi
echo "  ✓ accepting connections on 127.0.0.1:$PORT"

# 3. server version + basic SQL
echo "== 3. server identity =="
SERVER_VER=$("${PSQL[@]}" -tAc "SELECT version();")
echo "  $SERVER_VER"

echo "== 4. core SQL (types, JSON, arrays) =="
"${PSQL[@]}" -tAc "SELECT jsonb_build_object('a', ARRAY[1,2,3], 'ts', NOW())::text;" >/dev/null
echo "  ✓ jsonb + arrays + timestamps"

# 5. Discover and load every bundled contrib extension
echo "== 5. CREATE EXTENSION for every bundled contrib =="
EXT_DIR="$PG_PREFIX/share/extension"
if [ ! -d "$EXT_DIR" ]; then
    echo "::error::No share/extension dir at $EXT_DIR"
    exit 1
fi

FAILED_EXTS=()
LOADED_EXTS=0
for control in "$EXT_DIR"/*.control; do
    ext=$(basename "$control" .control)
    # plpgsql is preinstalled in template1; skip noisy "already exists" path.
    if [ "$ext" = "plpgsql" ]; then continue; fi
    if "${PSQL[@]}" -c "CREATE EXTENSION IF NOT EXISTS \"$ext\";" >/dev/null 2>&1; then
        LOADED_EXTS=$((LOADED_EXTS+1))
    else
        FAILED_EXTS+=("$ext")
    fi
done

echo "  ✓ loaded $LOADED_EXTS contrib extensions"
if [ ${#FAILED_EXTS[@]} -gt 0 ]; then
    echo "::error::Failed to load extension(s): ${FAILED_EXTS[*]}"
    # Show the actual server error for the first failure to aid debugging.
    "${PSQL[@]}" -c "CREATE EXTENSION IF NOT EXISTS \"${FAILED_EXTS[0]}\";" || true
    exit 1
fi

# 6. Exercise the most-used extensions (catches subtle .dylib breakage that
# CREATE EXTENSION alone wouldn't surface — many of these only load their
# .dylib lazily on first function call).
echo "== 6. exercise extension functions =="

assert_true() {
    local label="$1"
    local sql="$2"
    local result
    if ! result=$("${PSQL[@]}" -tAc "$sql" 2>&1); then
        echo "::error::$label query errored: $result"
        return 1
    fi
    if [ "$result" != "t" ]; then
        echo "::error::$label returned '$result', expected 't'"
        return 1
    fi
    echo "  ✓ $label"
}

assert_true "pg_trgm.similarity"         "SELECT similarity('postgres', 'postgresql') > 0;"
assert_true "pgcrypto.digest"            "SELECT length(digest('hello', 'sha256')) = 32;"
assert_true "hstore"                     "SELECT ('a=>1,b=>2'::hstore -> 'a') = '1';"
assert_true "citext"                     "SELECT 'Foo'::citext = 'foo';"
assert_true "ltree"                      "SELECT 'a.b.c'::ltree <@ 'a.b';"
assert_true "uuid-ossp.uuid_generate_v4" "SELECT uuid_generate_v4() IS NOT NULL;"
assert_true "cube"                       "SELECT cube(ARRAY[1,2,3]) IS NOT NULL;"
assert_true "intarray"                   "SELECT 1 = ANY(intarray_push_elem(ARRAY[2,3]::int[], 1));"
assert_true "fuzzystrmatch.levenshtein"  "SELECT levenshtein('kitten', 'sitting') = 3;"
assert_true "tablefunc.crosstab"         "SELECT EXISTS(SELECT 1 FROM pg_proc WHERE proname = 'crosstab');"
assert_true "dblink"                     "SELECT EXISTS(SELECT 1 FROM pg_proc WHERE proname = 'dblink_connect');"
assert_true "postgres_fdw"               "SELECT EXISTS(SELECT 1 FROM pg_foreign_data_wrapper WHERE fdwname = 'postgres_fdw');"

# 7. pgbench — real workload exercising storage, lock manager, WAL, planner.
echo "== 7. pgbench (scale 1, 10 seconds, 4 clients) =="
"$PG_PREFIX/bin/pgbench" -h 127.0.0.1 -p "$PORT" -U postgres -i -s 1 -q postgres 2>&1 | tail -3
"$PG_PREFIX/bin/pgbench" -h 127.0.0.1 -p "$PORT" -U postgres -T 10 -c 4 -j 2 postgres | grep -E '(tps|latency|number of transactions actually processed)' | head

# 8. pg_amcheck — heap + btree consistency over the whole cluster.
echo "== 8. pg_amcheck (heap + indexes) =="
if "$PG_PREFIX/bin/pg_amcheck" -h 127.0.0.1 -p "$PORT" -U postgres --all >/dev/null 2>&1; then
    echo "  ✓ no corruption"
else
    echo "::error::pg_amcheck reported corruption — re-running with output:"
    "$PG_PREFIX/bin/pg_amcheck" -h 127.0.0.1 -p "$PORT" -U postgres --all || true
    exit 1
fi

# 9. pg_dump → restore roundtrip exercises the client/library paths.
echo "== 9. pg_dump → restore roundtrip =="
DUMP_FILE=$(mktemp)
"$PG_PREFIX/bin/pg_dump" -h 127.0.0.1 -p "$PORT" -U postgres -d postgres > "$DUMP_FILE"
DUMP_SIZE=$(wc -c < "$DUMP_FILE" | tr -d ' ')
"${PSQL[@]}" -c "CREATE DATABASE pv_restoretest;" >/dev/null
"$PG_PREFIX/bin/psql" -h 127.0.0.1 -p "$PORT" -U postgres -d pv_restoretest -X -q < "$DUMP_FILE" >/dev/null

ORIG_ROWS=$("${PSQL[@]}" -tAc "SELECT count(*) FROM pgbench_accounts;")
RESTORED_ROWS=$("${PSQL[@]}" -d pv_restoretest -tAc "SELECT count(*) FROM pgbench_accounts;")
if [ "$ORIG_ROWS" != "$RESTORED_ROWS" ]; then
    echo "::error::Row count mismatch: original=$ORIG_ROWS restored=$RESTORED_ROWS"
    exit 1
fi
rm -f "$DUMP_FILE"
echo "  ✓ dumped $DUMP_SIZE bytes; restored row count matches ($ORIG_ROWS)"

echo ""
echo "== ALL CHECKS PASSED for $SERVER_VER =="
