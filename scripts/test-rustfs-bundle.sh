#!/bin/bash
# test-rustfs-bundle.sh — smoke test for a RustFS binary.
#
# Verifies that rustfs starts, accepts S3 API connections, and handles
# basic PUT/GET object operations.
#
# Usage:
#     scripts/test-rustfs-bundle.sh <path-to-rustfs-binary>
#
# Used by .github/workflows/build-artifacts.yml against the downloaded
# binary, and locally by passing an extracted CI artifact.
#
# Exit code: 0 = all checks passed; non-zero = at least one failed.

set -euo pipefail

RUSTFS="${1:?usage: $0 <path-to-rustfs-binary>}"
API_PORT="${RUSTFS_TEST_API_PORT:-19000}"
CONSOLE_PORT="${RUSTFS_TEST_CONSOLE_PORT:-19001}"

if [ ! -x "$RUSTFS" ]; then
    echo "::error::No executable at $RUSTFS"
    exit 1
fi

WORK_DIR="$(mktemp -d)"
RUSTFS_PID=""

ACCESS_KEY="rstfsadmin"
SECRET_KEY="rstfsadmin"

cleanup() {
    local rc=$?
    if [ -n "$RUSTFS_PID" ]; then
        kill "$RUSTFS_PID" 2>/dev/null || true
        wait "$RUSTFS_PID" 2>/dev/null || true
    fi
    if [ "$rc" -ne 0 ] && [ -s "$WORK_DIR/rustfs.log" ]; then
        echo "::group::rustfs.log (script exited $rc)" >&2
        cat "$WORK_DIR/rustfs.log" >&2
        echo "::endgroup::" >&2
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# 1. version
echo "== 1. rustfs version =="
RUSTFS_VER=$("$RUSTFS" --version 2>&1 || true)
echo "  $RUSTFS_VER"

# 2. start rustfs
echo "== 2. start rustfs =="
mkdir -p "$WORK_DIR/data"
RUSTFS_ACCESS_KEY="$ACCESS_KEY" \
RUSTFS_SECRET_KEY="$SECRET_KEY" \
"$RUSTFS" server "$WORK_DIR/data" \
    --address ":$API_PORT" \
    --console-enable \
    --console-address ":$CONSOLE_PORT" \
    >"$WORK_DIR/rustfs.log" 2>&1 &
RUSTFS_PID=$!

for i in $(seq 1 30); do
    if nc -z 127.0.0.1 "$API_PORT" 2>/dev/null; then
        break
    fi
    sleep 1
done

if ! nc -z 127.0.0.1 "$API_PORT" 2>/dev/null; then
    echo "::error::rustfs did not become ready within 30s"
    exit 1
fi
echo "  ✓ accepting connections (API :$API_PORT, Console :$CONSOLE_PORT)"

# 3. create a bucket
echo "== 3. create bucket =="
# Use curl with AWS SigV4 via --aws-sigv4.
# First, create a bucket named "pv-ci-test".
BUCKET="pv-ci-test"
curl -sf -X PUT \
    --aws-sigv4 "aws:amz:us-east-1:s3" \
    --user "$ACCESS_KEY:$SECRET_KEY" \
    "http://127.0.0.1:$API_PORT/$BUCKET" >/dev/null
echo "  ✓ bucket '$BUCKET' created"

# 4. PUT an object
echo "== 4. PUT object =="
OBJECT_CONTENT="hello from pv CI pipeline — $(date)"
echo -n "$OBJECT_CONTENT" > "$WORK_DIR/testfile.txt"
curl -sf -X PUT \
    --aws-sigv4 "aws:amz:us-east-1:s3" \
    --user "$ACCESS_KEY:$SECRET_KEY" \
    --upload-file "$WORK_DIR/testfile.txt" \
    "http://127.0.0.1:$API_PORT/$BUCKET/test-object.txt" >/dev/null
echo "  ✓ object uploaded"

# 5. GET the object back and verify content
echo "== 5. GET object and verify =="
GOT=$(curl -sf \
    --aws-sigv4 "aws:amz:us-east-1:s3" \
    --user "$ACCESS_KEY:$SECRET_KEY" \
    "http://127.0.0.1:$API_PORT/$BUCKET/test-object.txt")
if [ "$GOT" != "$OBJECT_CONTENT" ]; then
    echo "::error::GET returned unexpected content"
    echo "  expected: $OBJECT_CONTENT"
    echo "  got:      $GOT"
    exit 1
fi
echo "  ✓ content matches"

# 6. DELETE the object
echo "== 6. DELETE object =="
curl -sf -X DELETE \
    --aws-sigv4 "aws:amz:us-east-1:s3" \
    --user "$ACCESS_KEY:$SECRET_KEY" \
    "http://127.0.0.1:$API_PORT/$BUCKET/test-object.txt" >/dev/null
echo "  ✓ object deleted"

# 7. clean shutdown
echo "== 7. shutdown =="
kill "$RUSTFS_PID"
wait "$RUSTFS_PID" 2>/dev/null || true
RUSTFS_PID=""
echo "  ✓ clean shutdown"

echo ""
echo "== ALL CHECKS PASSED for $RUSTFS_VER =="
