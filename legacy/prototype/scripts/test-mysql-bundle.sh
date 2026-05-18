#!/bin/bash
# test-mysql-bundle.sh — integration test for a built MySQL bundle.
#
# Verifies that an extracted Oracle MySQL Community bundle (after our strip
# pass) actually works end-to-end: initialize, server startup, JSON +
# fulltext (ICU) + CRUD, mysqldump → restore roundtrip, and mysqlslap.
#
# Usage:
#     scripts/test-mysql-bundle.sh <path-to-extracted-bundle>
#
# The path argument should be the directory containing bin/, lib/, share/.
# Used by .github/workflows/build-artifacts.yml against the staged tree, and
# locally by passing an extracted CI artifact.
#
# Exit code: 0 = all checks passed; non-zero = at least one failed.

set -euo pipefail

MYSQL_PREFIX="${1:?usage: $0 <path-to-extracted-bundle>}"
PORT="${MYSQL_TEST_PORT:-33099}"

if [ ! -x "$MYSQL_PREFIX/bin/mysqld" ]; then
    echo "::error::No bin/mysqld at $MYSQL_PREFIX"
    exit 1
fi

WORK_DIR="$(mktemp -d)"
DATA_DIR="$WORK_DIR/data"
RUN_DIR="$WORK_DIR/run"
mkdir -p "$RUN_DIR"
MYSQL_PID=""
SOCK="$RUN_DIR/m.sock"

cleanup() {
    local rc=$?
    if [ -n "$MYSQL_PID" ]; then
        "$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root shutdown 2>/dev/null || \
            kill "$MYSQL_PID" 2>/dev/null || true
        wait "$MYSQL_PID" 2>/dev/null || true
    fi
    if [ "$rc" -ne 0 ] && [ -s "$WORK_DIR/mysqld.log" ]; then
        echo "::group::mysqld.log (script exited $rc)" >&2
        cat "$WORK_DIR/mysqld.log" >&2
        echo "::endgroup::" >&2
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

MYSQL=("$MYSQL_PREFIX/bin/mysql" --socket="$SOCK" -u root -N -B)

# 1. initialize
echo "== 1. mysqld --initialize-insecure =="
"$MYSQL_PREFIX/bin/mysqld" --initialize-insecure \
    --datadir="$DATA_DIR" --basedir="$MYSQL_PREFIX" --user="${USER:-$(id -un)}" 2>&1 | tail -5
echo "  ✓ datadir initialized at $DATA_DIR"

# 2. start mysqld in background
echo "== 2. start mysqld =="
"$MYSQL_PREFIX/bin/mysqld" \
    --datadir="$DATA_DIR" --basedir="$MYSQL_PREFIX" \
    --port="$PORT" --socket="$SOCK" --pid-file="$RUN_DIR/m.pid" \
    --mysqlx=OFF --innodb-buffer-pool-size=64M \
    >"$WORK_DIR/mysqld.log" 2>&1 &
MYSQL_PID=$!

for i in $(seq 1 30); do
    if "$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root ping >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

if ! "$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root ping >/dev/null 2>&1; then
    echo "::error::mysqld did not become ready within 30s"
    exit 1
fi
echo "  ✓ accepting connections on socket $SOCK (port $PORT)"

# 3. server identity
echo "== 3. server identity =="
SERVER_VER=$("${MYSQL[@]}" -e "SELECT VERSION();")
echo "  $SERVER_VER"

# 4. CRUD + JSON + fulltext (exercises ICU + plugin loading)
echo "== 4. CRUD + JSON + fulltext =="
"${MYSQL[@]}" <<'SQL'
CREATE DATABASE pv_test;
USE pv_test;
CREATE TABLE t (
    id INT PRIMARY KEY,
    name VARCHAR(64),
    payload JSON,
    FULLTEXT KEY ft_name (name)
) ENGINE=InnoDB;
INSERT INTO t VALUES
    (1, 'one apple',   '{"k":1}'),
    (2, 'two bananas', '{"k":2}');
SQL

JSON_RESULT=$("${MYSQL[@]}" -e "SELECT JSON_EXTRACT(payload,'\$.k') FROM pv_test.t WHERE id=1;")
if [ "$JSON_RESULT" != "1" ]; then
    echo "::error::JSON_EXTRACT returned '$JSON_RESULT', expected '1'"
    exit 1
fi
echo "  ✓ JSON_EXTRACT"

FT_HITS=$("${MYSQL[@]}" -e "SELECT COUNT(*) FROM pv_test.t WHERE MATCH(name) AGAINST('apple');")
if [ "$FT_HITS" -lt 1 ]; then
    echo "::error::Fulltext MATCH returned $FT_HITS hits, expected >=1"
    exit 1
fi
echo "  ✓ Fulltext MATCH (ICU)"

# 5. mysqldump → restore roundtrip
echo "== 5. mysqldump → restore roundtrip =="
DUMP_FILE="$WORK_DIR/dump.sql"
# --set-gtid-purged=OFF: skip the "SET @@GLOBAL.gtid_purged = ...;" preamble.
# Restoring it into the same server (where GTID_EXECUTED is already non-empty)
# fails on MySQL 9.x with ER_CANT_SET_GTID_PURGED_DUE_TO_OVERLAP.
"$MYSQL_PREFIX/bin/mysqldump" --socket="$SOCK" -u root --set-gtid-purged=OFF \
    --databases pv_test > "$DUMP_FILE"
DUMP_SIZE=$(wc -c < "$DUMP_FILE" | tr -d ' ')

"${MYSQL[@]}" -e "CREATE DATABASE pv_test_restored;"
"$MYSQL_PREFIX/bin/mysql" --socket="$SOCK" -u root pv_test_restored \
    < <(sed 's/`pv_test`/`pv_test_restored`/g' "$DUMP_FILE")

ORIG_ROWS=$("${MYSQL[@]}" -e "SELECT COUNT(*) FROM pv_test.t;")
REST_ROWS=$("${MYSQL[@]}" -e "SELECT COUNT(*) FROM pv_test_restored.t;")
if [ "$ORIG_ROWS" != "$REST_ROWS" ]; then
    echo "::error::Row count mismatch: original=$ORIG_ROWS restored=$REST_ROWS"
    exit 1
fi
echo "  ✓ dumped $DUMP_SIZE bytes; restored row count matches ($ORIG_ROWS)"

# 6. mysqlslap — exercises connection handling + threading
echo "== 6. mysqlslap (4 clients × 20 iterations) =="
"$MYSQL_PREFIX/bin/mysqlslap" --socket="$SOCK" -u root \
    --concurrency=4 --iterations=20 \
    --auto-generate-sql --auto-generate-sql-load-type=mixed \
    --number-of-queries=200 2>&1 | grep -E '(seconds to run|Average|Minimum|Maximum)' | head

# 7. clean shutdown
echo "== 7. shutdown =="
"$MYSQL_PREFIX/bin/mysqladmin" --socket="$SOCK" -u root shutdown
wait "$MYSQL_PID" 2>/dev/null || true
MYSQL_PID=""
echo "  ✓ clean shutdown"

echo ""
echo "== ALL CHECKS PASSED for $SERVER_VER =="
