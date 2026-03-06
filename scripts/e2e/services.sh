#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "=== E2E: Service Lifecycle ==="

# Add Redis.
echo "--- pv service add redis ---"
OUTPUT=$(pv service add redis 2>&1)
assert_contains "$OUTPUT" "running" "service add redis should show running"
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
assert_contains "$OUTPUT" "mysql:latest" "should pull mysql:latest"
echo "OK: mysql added with latest tag"

# Destroy MySQL — no prompt, data deleted.
echo "--- pv service destroy mysql ---"
OUTPUT=$(pv service destroy mysql 2>&1)
assert_contains "$OUTPUT" "destroyed" "destroy should show destroyed"
echo "OK: mysql destroyed (no prompt)"

# Service env.
echo "--- pv service add redis (for env test) ---"
pv service add redis 2>&1
OUTPUT=$(pv service env redis 2>&1)
assert_contains "$OUTPUT" "REDIS_HOST" "env should show REDIS_HOST"
assert_contains "$OUTPUT" "REDIS_PORT" "env should show REDIS_PORT"
echo "OK: service env"

# Cleanup.
pv service destroy redis 2>&1 || true

echo "=== E2E: Service Lifecycle PASSED ==="
