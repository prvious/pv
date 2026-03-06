#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "=== E2E: Service Auto-Wiring ==="

# Start MySQL + Redis.
echo "--- Adding MySQL + Redis ---"
pv service add mysql 2>&1
pv service add redis 2>&1

# Verify both exist.
OUTPUT=$(pv service list 2>&1)
assert_contains "$OUTPUT" "mysql" "should list mysql"
assert_contains "$OUTPUT" "redis" "should list redis"
echo "OK: services added"

# Create a Laravel-like fixture with .env.
FIXTURE_DIR="/tmp/e2e-service-link"
rm -rf "$FIXTURE_DIR"
mkdir -p "$FIXTURE_DIR/public"

cat > "$FIXTURE_DIR/.env" <<'ENVEOF'
APP_NAME=TestApp
DB_CONNECTION=mysql
DB_HOST=localhost
DB_PORT=3306
DB_DATABASE=testapp
DB_USERNAME=root
DB_PASSWORD=
REDIS_HOST=127.0.0.1
REDIS_PORT=6379
ENVEOF

# Create a minimal composer.json for detection.
cat > "$FIXTURE_DIR/composer.json" <<'JSONEOF'
{
    "require": {
        "php": "^8.4",
        "laravel/framework": "^11.0"
    }
}
JSONEOF

echo "<?php echo 'hello';" > "$FIXTURE_DIR/public/index.php"

# Link the project.
echo "--- pv link ---"
OUTPUT=$(pv link "$FIXTURE_DIR" --name e2e-service-link 2>&1)
assert_contains "$OUTPUT" "Linked" "should show linked"
echo "OK: project linked"

# Verify the project is in the list.
OUTPUT=$(pv list 2>&1)
assert_contains "$OUTPUT" "e2e-service-link" "project should appear in list"
echo "OK: project listed"

# Test env output for mysql.
OUTPUT=$(pv service env mysql 2>&1)
assert_contains "$OUTPUT" "DB_CONNECTION" "env should show DB_CONNECTION"
assert_contains "$OUTPUT" "DB_HOST" "env should show DB_HOST"
echo "OK: mysql env vars"

# Cleanup.
pv unlink e2e-service-link 2>&1 || true
pv service destroy mysql 2>&1 || true
pv service destroy redis 2>&1 || true
rm -rf "$FIXTURE_DIR"

echo "=== E2E: Service Auto-Wiring PASSED ==="
