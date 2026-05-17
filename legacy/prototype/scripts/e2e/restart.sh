#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

sudo -E pv restart
sleep 3

setup_curl

curl_site "e2e-php.test" "php works"
echo "OK: php site works after restart"

curl_site "e2e-php83.test" "php83 works"
echo "OK: php83 site works after restart (multi-version intact)"
