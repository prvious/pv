#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "=== E2E: Service Lifecycle ==="

# Add Redis.
echo "--- pv service add redis ---"
OUTPUT=$(pv service add redis 2>&1)
assert_contains "$OUTPUT" "Redis" "service add redis should show Redis"
assert_contains "$OUTPUT" "6379" "service add redis should show port 6379"
echo "OK: redis added"

# Add same Redis again — should say already added.
echo "--- pv service add redis (duplicate) ---"
OUTPUT=$(pv service add redis 2>&1)
assert_contains "$OUTPUT" "already added" "duplicate add should say already added"
echo "OK: duplicate detected"

# List services.
echo "--- pv service list ---"
OUTPUT=$(pv service list 2>&1)
assert_contains "$OUTPUT" "redis" "service list should show redis"
assert_contains "$OUTPUT" "6379" "service list should show port"
echo "OK: service list"

# Service status.
echo "--- pv service status redis ---"
OUTPUT=$(pv service status redis 2>&1)
assert_contains "$OUTPUT" "Redis" "status should show Redis"
assert_contains "$OUTPUT" "6379" "status should show port"
echo "OK: service status"

# Stop service.
echo "--- pv service stop redis ---"
OUTPUT=$(pv service stop redis 2>&1)
assert_contains "$OUTPUT" "stopped" "stop should show stopped"
echo "OK: service stopped"

# Start service.
echo "--- pv service start redis ---"
OUTPUT=$(pv service start redis 2>&1)
assert_contains "$OUTPUT" "started" "start should show started"
echo "OK: service started"

# Remove service (keeps data).
echo "--- pv service remove redis ---"
OUTPUT=$(pv service remove redis 2>&1)
assert_contains "$OUTPUT" "removed" "remove should show removed"
assert_contains "$OUTPUT" "preserved" "remove should mention data preserved"
echo "OK: service removed"

# Add MySQL with default version (latest).
echo "--- pv service add mysql ---"
OUTPUT=$(pv service add mysql 2>&1)
assert_contains "$OUTPUT" "MySQL" "should show MySQL"
assert_contains "$OUTPUT" "latest" "should show latest version"
echo "OK: mysql added with latest tag"

# Add MySQL with specific version.
echo "--- pv service add mysql 8.0.32 ---"
OUTPUT=$(pv service add mysql 8.0.32 2>&1)
assert_contains "$OUTPUT" "MySQL" "should show MySQL"
assert_contains "$OUTPUT" "33032" "should show port 33032"
echo "OK: mysql 8.0.32 added"

# Destroy MySQL — no prompt, data deleted.
echo "--- pv service destroy mysql ---"
OUTPUT=$(pv service destroy mysql 2>&1)
assert_contains "$OUTPUT" "destroyed" "destroy should show destroyed"
echo "OK: mysql:latest destroyed (no prompt)"

echo "--- pv service destroy mysql:8.0.32 ---"
OUTPUT=$(pv service destroy mysql:8.0.32 2>&1)
assert_contains "$OUTPUT" "destroyed" "destroy should show destroyed"
echo "OK: mysql:8.0.32 destroyed (no prompt)"

# Add postgres with version.
echo "--- pv service add postgres 16 ---"
OUTPUT=$(pv service add postgres 16 2>&1)
assert_contains "$OUTPUT" "PostgreSQL" "should show PostgreSQL"
assert_contains "$OUTPUT" "54016" "should show port 54016"
echo "OK: postgres 16 added"
pv service destroy postgres:16 2>&1 || true

# Add RustFS.
echo "--- pv service add rustfs ---"
OUTPUT=$(pv service add rustfs 2>&1)
assert_contains "$OUTPUT" "RustFS" "should show RustFS"
assert_contains "$OUTPUT" "9000" "should show port 9000"
assert_contains "$OUTPUT" "9001" "should show console port 9001"
echo "OK: rustfs added"
pv service destroy rustfs 2>&1 || true

# Service env.
echo "--- pv service env (redis) ---"
pv service add redis 2>&1 > /dev/null
OUTPUT=$(pv service env redis 2>&1)
assert_contains "$OUTPUT" "REDIS_HOST" "env should show REDIS_HOST"
assert_contains "$OUTPUT" "REDIS_PORT" "env should show REDIS_PORT"
echo "OK: service env"

# Error: unknown service.
echo "--- pv service add unknown ---"
if pv service add mongodb 2>&1; then
  echo "FAIL: should have failed for unknown service"
  exit 1
fi
echo "OK: unknown service rejected"

# Error: service not found for status.
echo "--- pv service status nonexistent ---"
if pv service status nonexistent 2>&1; then
  echo "FAIL: should have failed for nonexistent service"
  exit 1
fi
echo "OK: nonexistent service rejected"

# Cleanup.
pv service destroy redis 2>&1 || true

# Verify list is empty after cleanup.
echo "--- pv service list (empty) ---"
OUTPUT=$(pv service list 2>&1)
assert_contains "$OUTPUT" "No services" "empty list should show helpful message"
echo "OK: empty list message"

echo "=== E2E: Service Lifecycle PASSED ==="
