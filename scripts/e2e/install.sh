#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

pv install --php 8.4
echo "--- install output above ---"

pv php install 8.3
echo "--- PHP 8.3 installed ---"
