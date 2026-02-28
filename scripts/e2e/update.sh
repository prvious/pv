#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

pv update
ls ~/.pv/bin/mago || { echo "FAIL: mago missing after update"; exit 1; }
ls ~/.pv/bin/composer || { echo "FAIL: composer missing after update"; exit 1; }
echo "OK: pv update completed"
