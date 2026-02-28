#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

sudo -E pv start &
sleep 8

echo "==> pv status"
STATUS=$(sudo -E pv status)
echo "$STATUS"
assert_contains "$STATUS" "running" "server not running"
assert_contains "$STATUS" "8.4 (global)" "global PHP not shown"
assert_contains "$STATUS" "5 linked" "wrong site count"

# Version Caddyfile for 8.3 generated at start time
echo "--- php-8.3.Caddyfile ---"
cat ~/.pv/config/php-8.3.Caddyfile
grep -q "8830" ~/.pv/config/php-8.3.Caddyfile || { echo "FAIL: 8.3 Caddyfile missing port 8830"; exit 1; }

setup_curl

curl_site "e2e-static.test" "static works"
curl_site "e2e-php.test" "php works"
curl_site "e2e-laravel.test" "laravel works"
curl_site "e2e-octane.test" "octane works"
echo "OK: octane site (laravel-octane detection)"
curl_site "e2e-php83.test" "php83 works"
echo "OK: php83 site (multi-version via reverse proxy on port 8830)"
