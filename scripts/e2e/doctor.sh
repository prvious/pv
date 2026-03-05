#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Run doctor with sudo -E so it sees the same HOME and PID file as the server
# (the server was started with sudo -E pv start &).
echo "==> Run pv doctor (server running)"
OUTPUT=$(sudo -E pv doctor 2>&1 || true)
echo "$OUTPUT"

# Binaries should be found.
assert_contains "$OUTPUT" "frankenphp + php" "PHP binaries not detected"
assert_contains "$OUTPUT" "Composer" "Composer not detected"

# Environment checks.
assert_contains "$OUTPUT" "PATH" "PATH check missing"
assert_contains "$OUTPUT" "PHP shim" "PHP shim check missing"

# Composer isolation checks.
assert_contains "$OUTPUT" "Composer home directory" "Composer home directory check missing"
assert_contains "$OUTPUT" "COMPOSER_HOME isolated" "COMPOSER_HOME isolation check missing"

# Network checks.
assert_contains "$OUTPUT" "DNS resolver" "DNS resolver check missing"
assert_contains "$OUTPUT" "CA certificate" "CA certificate check missing"

# Server checks (server should be running at this point).
assert_contains "$OUTPUT" "Running" "Server not detected as running"

# Projects section.
assert_contains "$OUTPUT" "Projects" "Projects section missing"

echo "==> Verify doctor detects missing project directory"
# Link a project with a nonexistent path by writing directly to registry.
REGISTRY=~/.pv/data/registry.json
BACKUP=$(cat "$REGISTRY")

# Add a fake project to the registry.
echo "$BACKUP" | python3 -c "
import json, sys
reg = json.load(sys.stdin)
reg['projects'].append({'name': 'ghost-app', 'path': '/nonexistent/ghost', 'type': 'laravel', 'php': ''})
json.dump(reg, sys.stdout, indent=2)
" > "$REGISTRY"

OUTPUT=$(sudo -E pv doctor 2>&1 || true)
echo "$OUTPUT"
assert_contains "$OUTPUT" "ghost-app" "ghost project not checked"
assert_contains "$OUTPUT" "directory missing" "missing directory not detected"
assert_contains "$OUTPUT" "pv unlink ghost-app" "fix suggestion for missing project not shown"

# Restore original registry.
echo "$BACKUP" > "$REGISTRY"

echo "OK: pv doctor working correctly"
