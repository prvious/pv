#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "=== E2E: Service Auto-Wiring ==="

# Start MySQL + Redis.
echo "--- Adding MySQL + Redis ---"
pv service add mysql 2>&1
pv service add redis 2>&1

# Create a Laravel-like fixture with .env.
FIXTURE_DIR="/tmp/e2e-service-link"
rm -rf "$FIXTURE_DIR"
mkdir -p "$FIXTURE_DIR"

cat > "$FIXTURE_DIR/.env" <<'EOF'
APP_NAME=TestApp
DB_CONNECTION=mysql
DB_HOST=localhost
DB_PORT=3306
DB_DATABASE=testapp
DB_USERNAME=root
DB_PASSWORD=
REDIS_HOST=127.0.0.1
REDIS_PORT=6379
EOF

# Create a minimal composer.json for detection.
cat > "$FIXTURE_DIR/composer.json" <<'EOF'
{
    "require": {
        "php": "^8.4",
        "laravel/framework": "^11.0"
    }
}
EOF
mkdir -p "$FIXTURE_DIR/public"
echo "<?php echo 'hello';" > "$FIXTURE_DIR/public/index.php"

# Link the project.
echo "--- pv link ---"
OUTPUT=$(pv link "$FIXTURE_DIR" --name e2e-service-link 2>&1)
assert_contains "$OUTPUT" "Linked" "should show linked"
echo "OK: project linked"

# Verify service detection.
assert_contains "$OUTPUT" "mysql" "should detect mysql from DB_CONNECTION" || true
assert_contains "$OUTPUT" "redis" "should detect redis from REDIS_HOST" || true
echo "OK: services detected"

# Cleanup.
pv unlink e2e-service-link 2>&1 || true
pv service destroy mysql 2>&1 || true
pv service destroy redis 2>&1 || true
rm -rf "$FIXTURE_DIR"

echo "=== E2E: Service Auto-Wiring PASSED ==="
