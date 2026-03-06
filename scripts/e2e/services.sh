#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# run_pv: run a pv command, capture output, print on failure.
run_pv() {
  local output
  if output=$("$@" 2>&1); then
    echo "$output"
  else
    echo "Command failed: $*"
    echo "$output"
    exit 1
  fi
}

echo "=== E2E: Service Lifecycle ==="

# Add Redis.
echo "--- pv service add redis ---"
OUTPUT=$(run_pv pv service add redis)
assert_contains "$OUTPUT" "Redis" "service add redis should show Redis"
assert_contains "$OUTPUT" "6379" "service add redis should show port 6379"
echo "OK: redis added"

# Add same Redis again — should say already added.
echo "--- pv service add redis (duplicate) ---"
OUTPUT=$(run_pv pv service add redis)
assert_contains "$OUTPUT" "already added" "duplicate add should say already added"
echo "OK: duplicate detected"

# List services.
echo "--- pv service list ---"
OUTPUT=$(run_pv pv service list)
assert_contains "$OUTPUT" "redis" "service list should show redis"
assert_contains "$OUTPUT" "6379" "service list should show port"
echo "OK: service list"

# Service status.
echo "--- pv service status redis ---"
OUTPUT=$(run_pv pv service status redis)
assert_contains "$OUTPUT" "Redis" "status should show Redis"
assert_contains "$OUTPUT" "6379" "status should show port"
echo "OK: service status"

# Stop service.
echo "--- pv service stop redis ---"
OUTPUT=$(run_pv pv service stop redis)
assert_contains "$OUTPUT" "stopped" "stop should show stopped"
echo "OK: service stopped"

# Start service.
echo "--- pv service start redis ---"
OUTPUT=$(run_pv pv service start redis)
assert_contains "$OUTPUT" "started" "start should show started"
echo "OK: service started"

# Remove service (keeps data).
echo "--- pv service remove redis ---"
OUTPUT=$(run_pv pv service remove redis)
assert_contains "$OUTPUT" "removed" "remove should show removed"
assert_contains "$OUTPUT" "preserved" "remove should mention data preserved"
echo "OK: service removed"

# Add MySQL with default version (latest).
echo "--- pv service add mysql ---"
OUTPUT=$(run_pv pv service add mysql)
assert_contains "$OUTPUT" "MySQL" "should show MySQL"
assert_contains "$OUTPUT" "latest" "should show latest version"
echo "OK: mysql added with latest tag"

# Add MySQL with specific version.
echo "--- pv service add mysql 8.0.32 ---"
OUTPUT=$(run_pv pv service add mysql 8.0.32)
assert_contains "$OUTPUT" "MySQL" "should show MySQL"
assert_contains "$OUTPUT" "33032" "should show port 33032"
echo "OK: mysql 8.0.32 added"

# Destroy MySQL — no prompt, data deleted.
echo "--- pv service destroy mysql ---"
OUTPUT=$(run_pv pv service destroy mysql)
assert_contains "$OUTPUT" "destroyed" "destroy should show destroyed"
echo "OK: mysql:latest destroyed (no prompt)"

echo "--- pv service destroy mysql:8.0.32 ---"
OUTPUT=$(run_pv pv service destroy mysql:8.0.32)
assert_contains "$OUTPUT" "destroyed" "destroy should show destroyed"
echo "OK: mysql:8.0.32 destroyed (no prompt)"

# Add postgres with version.
echo "--- pv service add postgres 16 ---"
OUTPUT=$(run_pv pv service add postgres 16)
assert_contains "$OUTPUT" "PostgreSQL" "should show PostgreSQL"
assert_contains "$OUTPUT" "54016" "should show port 54016"
echo "OK: postgres 16 added"
run_pv pv service destroy postgres:16 > /dev/null

# Add RustFS.
echo "--- pv service add rustfs ---"
OUTPUT=$(run_pv pv service add rustfs)
assert_contains "$OUTPUT" "RustFS" "should show RustFS"
assert_contains "$OUTPUT" "9000" "should show port 9000"
assert_contains "$OUTPUT" "9001" "should show console port 9001"
echo "OK: rustfs added"
run_pv pv service destroy rustfs > /dev/null

# Service env.
echo "--- pv service env (redis) ---"
run_pv pv service add redis > /dev/null
OUTPUT=$(run_pv pv service env redis)
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
run_pv pv service destroy redis > /dev/null

# Verify list is empty after cleanup.
echo "--- pv service list (empty) ---"
OUTPUT=$(run_pv pv service list)
assert_contains "$OUTPUT" "No services" "empty list should show helpful message"
echo "OK: empty list message"

echo "=== E2E: Service Lifecycle PASSED ==="
