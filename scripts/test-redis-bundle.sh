#!/bin/bash
# test-redis-bundle.sh — smoke test for a built Redis bundle.
#
# Verifies that redis-server starts, accepts connections, handles basic
# SET/GET operations, and shuts down cleanly.
#
# Usage:
#     scripts/test-redis-bundle.sh <path-to-staging-dir>
#
# The path argument should be the directory containing redis-server and
# redis-cli. Used by .github/workflows/build-artifacts.yml against the
# staged binaries, and locally by passing an extracted CI artifact.
#
# Exit code: 0 = all checks passed; non-zero = at least one failed.

set -euo pipefail

STAGING="${1:?usage: $0 <path-to-staging-dir>}"
PORT="${REDIS_TEST_PORT:-16399}"

SERVER="$STAGING/redis-server"
CLI="$STAGING/redis-cli"

if [ ! -x "$SERVER" ]; then
    echo "::error::No redis-server at $SERVER"
    exit 1
fi
if [ ! -x "$CLI" ]; then
    echo "::error::No redis-cli at $CLI"
    exit 1
fi

WORK_DIR="$(mktemp -d)"
REDIS_PID=""

cleanup() {
    local rc=$?
    # Try to read PID from pidfile if we don't have it yet (daemonized server).
    if [ -z "$REDIS_PID" ] && [ -f "$WORK_DIR/redis.pid" ]; then
        REDIS_PID=$(cat "$WORK_DIR/redis.pid" 2>/dev/null || true)
    fi
    if [ -n "$REDIS_PID" ]; then
        "$CLI" -p "$PORT" SHUTDOWN NOSAVE 2>/dev/null || \
            kill "$REDIS_PID" 2>/dev/null || true
        wait "$REDIS_PID" 2>/dev/null || true
    fi
    if [ "$rc" -ne 0 ] && [ -s "$WORK_DIR/redis.log" ]; then
        echo "::group::redis.log (script exited $rc)" >&2
        cat "$WORK_DIR/redis.log" >&2
        echo "::endgroup::" >&2
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# 1. server version
echo "== 1. redis-server version =="
SERVER_VER=$("$SERVER" --version | head -1)
echo "  $SERVER_VER"

# 2. start redis-server
echo "== 2. start redis-server =="
"$SERVER" \
    --port "$PORT" \
    --bind 127.0.0.1 \
    --dir "$WORK_DIR" \
    --save "" \
    --appendonly no \
    --daemonize yes \
    --logfile "$WORK_DIR/redis.log" \
    --pidfile "$WORK_DIR/redis.pid"

# daemonize writes the pid file; read it so cleanup can target it.
for i in $(seq 1 15); do
    if "$CLI" -p "$PORT" PING 2>/dev/null | grep -q PONG; then
        break
    fi
    sleep 1
done

if ! "$CLI" -p "$PORT" PING 2>/dev/null | grep -q PONG; then
    echo "::error::redis-server did not become ready within 15s"
    exit 1
fi
REDIS_PID=$(cat "$WORK_DIR/redis.pid" 2>/dev/null || true)
echo "  ✓ accepting connections on 127.0.0.1:$PORT (pid $REDIS_PID)"

# 3. PING
echo "== 3. PING =="
PONG=$("$CLI" -p "$PORT" PING)
if [ "$PONG" != "PONG" ]; then
    echo "::error::PING returned '$PONG', expected 'PONG'"
    exit 1
fi
echo "  ✓ PONG"

# 4. SET / GET roundtrip
echo "== 4. SET / GET roundtrip =="
"$CLI" -p "$PORT" SET pv:test:key "hello-from-ci" >/dev/null
GOT=$("$CLI" -p "$PORT" GET pv:test:key)
if [ "$GOT" != "hello-from-ci" ]; then
    echo "::error::GET returned '$GOT', expected 'hello-from-ci'"
    exit 1
fi
echo "  ✓ SET/GET roundtrip"

# 5. basic data structures (hash, list)
echo "== 5. data structures (hash, list) =="
"$CLI" -p "$PORT" HSET pv:test:hash field1 val1 field2 val2 >/dev/null
HGET=$("$CLI" -p "$PORT" HGET pv:test:hash field1)
if [ "$HGET" != "val1" ]; then
    echo "::error::HGET returned '$HGET', expected 'val1'"
    exit 1
fi

"$CLI" -p "$PORT" RPUSH pv:test:list a b c >/dev/null
LLEN=$("$CLI" -p "$PORT" LLEN pv:test:list)
if [ "$LLEN" != "3" ]; then
    echo "::error::LLEN returned '$LLEN', expected '3'"
    exit 1
fi
echo "  ✓ HSET/HGET + RPUSH/LLEN"

# 6. INFO (exercises stats subsystem)
echo "== 6. INFO server =="
REDIS_VERSION=$("$CLI" -p "$PORT" INFO server | grep "redis_version:" | cut -d: -f2 | tr -d '[:space:]')
echo "  redis_version=$REDIS_VERSION"

# 7. clean shutdown
echo "== 7. shutdown =="
"$CLI" -p "$PORT" SHUTDOWN NOSAVE
wait "$REDIS_PID" 2>/dev/null || true
REDIS_PID=""
echo "  ✓ clean shutdown"

echo ""
echo "== ALL CHECKS PASSED for Redis $REDIS_VERSION =="
