#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Start with only PHP 8.4 (8.3 was removed, php83+static unlinked)
sudo -E pv start &
sleep 8

echo "==> pv status"
sudo -E pv status

setup_curl

curl_site "e2e-php.test" "php works"
curl_site "e2e-laravel.test" "laravel works"
curl_site "e2e-octane.test" "octane works"
curl_site "e2e-dynamic.test" "dynamic works"

sudo -E pv stop
echo "OK: server verified after PHP changes"
