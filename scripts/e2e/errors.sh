#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# 1. Link non-existent path
assert_fails pv link /tmp/does-not-exist
echo "OK: link non-existent path -> error"

# 2. Link duplicate name
assert_fails pv link /tmp/e2e-php --name e2e-php
echo "OK: duplicate link -> error"

# 3. Remove global PHP version
assert_fails pv php remove 8.4
echo "OK: remove global PHP -> error"

# 4. Remove PHP version with dependent project
assert_fails pv php remove 8.3
echo "OK: remove PHP with dependent project -> error"

# 5. Install already-installed PHP
assert_fails pv php install 8.4
echo "OK: install already-installed PHP -> error"

# 6. Use non-installed PHP version
assert_fails pv use php:9.9
echo "OK: use non-installed PHP -> error"

# 7. Install invalid PHP version format
assert_fails pv php install abc
echo "OK: install invalid format -> error"

# 8. Unlink non-existent project
assert_fails pv unlink nonexistent-project
echo "OK: unlink non-existent project -> error"

echo "All 8 error cases passed"
