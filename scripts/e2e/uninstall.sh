#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# Stop server first (may already be stopped from earlier phase).
sudo -E pv stop 2>/dev/null || true
sleep 1

# Record state before uninstall.
echo "==> Pre-uninstall checks"
[ -d ~/.pv ] || { echo "FAIL: ~/.pv does not exist before uninstall"; exit 1; }
echo "OK: ~/.pv exists"
PV_BIN=$(which pv)
echo "pv binary at: $PV_BIN"

# Run uninstall by piping "uninstall" and "n" (decline auth backup).
# Don't wrap in sudo — pv handles sudo internally via sudo -n.
printf 'uninstall\nn\n' | pv uninstall

# Verify ~/.pv is gone.
echo "==> Post-uninstall checks"
if [ -d ~/.pv ]; then
  echo "FAIL: ~/.pv still exists after uninstall"
  ls -la ~/.pv/
  exit 1
fi
echo "OK: ~/.pv removed"

# Verify launchd plist is gone.
PLIST="$HOME/Library/LaunchAgents/dev.prvious.pv.plist"
if [ -f "$PLIST" ]; then
  echo "FAIL: launchd plist still exists"
  exit 1
fi
echo "OK: launchd plist removed"

# Verify DNS resolver is gone.
if [ -f /etc/resolver/test ]; then
  echo "FAIL: /etc/resolver/test still exists"
  exit 1
fi
echo "OK: DNS resolver removed"

# Verify no pv processes running.
if pgrep -f frankenphp > /dev/null 2>&1; then
  echo "FAIL: frankenphp processes still running"
  pgrep -af frankenphp
  exit 1
fi
echo "OK: no frankenphp processes"

# Verify pv binary is gone.
if [ -f "$PV_BIN" ]; then
  echo "FAIL: pv binary still exists at $PV_BIN"
  exit 1
fi
echo "OK: pv binary removed"

echo "==> pv uninstall e2e passed"
