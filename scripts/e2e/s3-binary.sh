#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> Phase: S3 binary service (RustFS) lifecycle"

# e2e tests use foreground mode with sudo (previous phases leave root-owned
# config dirs; only root can clean and regenerate them).
sudo -E pv start >/tmp/pv-s3-e2e.log 2>&1 &
START_PID=$!
sleep 8

cleanup() {
  sudo -E pv stop >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> service:add s3"
sudo -E pv service:add s3 || { echo "FAIL: pv service:add s3 failed"; exit 1; }

echo "==> Verify rustfs binary exists"
test -x "$HOME/.pv/internal/bin/rustfs" || { echo "FAIL: rustfs binary not installed"; exit 1; }
echo "OK: rustfs binary at ~/.pv/internal/bin/rustfs"

echo "==> Verify daemon-status.json lists rustfs"
for i in $(seq 1 20); do
    if grep -q '"rustfs"' "$HOME/.pv/daemon-status.json" 2>/dev/null; then break; fi
    sleep 1
done
grep -q '"rustfs"' "$HOME/.pv/daemon-status.json" 2>/dev/null || {
    echo "FAIL: daemon-status.json does not contain rustfs entry";
    cat "$HOME/.pv/daemon-status.json" 2>/dev/null || echo "(file missing)";
    exit 1;
}
echo "OK: daemon-status.json advertises rustfs"

echo "==> Verify port 9000 is reachable"
for i in $(seq 1 20); do
    if nc -z 127.0.0.1 9000 2>/dev/null; then break; fi
    sleep 1
done
nc -z 127.0.0.1 9000 || { echo "FAIL: port 9000 not reachable after service:add"; exit 1; }
echo "OK: port 9000 reachable"

echo "==> service:stop s3"
sudo -E pv service:stop s3
sleep 2
if nc -z 127.0.0.1 9000 2>/dev/null; then
    echo "FAIL: port 9000 still answering after service:stop"
    exit 1
fi
echo "OK: port 9000 silent after service:stop"

echo "==> service:start s3"
sudo -E pv service:start s3
for i in $(seq 1 20); do
    if nc -z 127.0.0.1 9000 2>/dev/null; then break; fi
    sleep 1
done
nc -z 127.0.0.1 9000 || { echo "FAIL: port 9000 not reachable after service:start"; exit 1; }
echo "OK: port 9000 reachable after service:start"

echo "==> service:destroy s3"
sudo -E pv service:destroy s3
test ! -f "$HOME/.pv/internal/bin/rustfs" || { echo "FAIL: rustfs binary not deleted after destroy"; exit 1; }
test ! -d "$HOME/.pv/services/s3/latest/data" || { echo "FAIL: data dir not deleted after destroy"; exit 1; }
echo "OK: binary and data removed"

echo "==> pv stop"
sudo -E pv stop || true
trap - EXIT

echo "OK: S3 binary service lifecycle passed"
