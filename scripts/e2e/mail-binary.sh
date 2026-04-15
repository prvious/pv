#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> Phase: Mail binary service (Mailpit) lifecycle"

# Start pv in the background so we have a live daemon for service: commands.
pv start >/tmp/pv-mail-e2e.log 2>&1 &
START_PID=$!
sleep 3

cleanup() {
  kill "$START_PID" 2>/dev/null || true
  pv stop >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> service:add mail"
pv service:add mail || { echo "FAIL: pv service:add mail failed"; exit 1; }

echo "==> Verify mailpit binary exists"
test -x "$HOME/.pv/internal/bin/mailpit" || { echo "FAIL: mailpit binary not installed"; exit 1; }
echo "OK: mailpit binary at ~/.pv/internal/bin/mailpit"

echo "==> Verify daemon-status.json lists mailpit"
test -f "$HOME/.pv/daemon-status.json" || { echo "FAIL: daemon-status.json missing"; exit 1; }
grep -q '"mailpit"' "$HOME/.pv/daemon-status.json" || {
    echo "FAIL: daemon-status.json does not contain mailpit entry";
    cat "$HOME/.pv/daemon-status.json";
    exit 1;
}
echo "OK: daemon-status.json advertises mailpit"

echo "==> Verify HTTP /livez on port 8025 responds"
for i in $(seq 1 20); do
    if curl -fsS http://127.0.0.1:8025/livez 2>/dev/null; then break; fi
    sleep 1
done
curl -fsS http://127.0.0.1:8025/livez || { echo "FAIL: /livez not reachable after service:add"; exit 1; }
echo "OK: /livez reachable on port 8025"

echo "==> Verify SMTP port 1025 is reachable"
nc -z 127.0.0.1 1025 || { echo "FAIL: SMTP port 1025 not reachable after service:add"; exit 1; }
echo "OK: SMTP port 1025 reachable"

echo "==> service:stop mail"
pv service:stop mail
sleep 2
if curl -fsS http://127.0.0.1:8025/livez 2>/dev/null; then
    echo "FAIL: /livez still answering after service:stop"
    exit 1
fi
echo "OK: /livez silent after service:stop"

echo "==> service:start mail"
pv service:start mail
for i in $(seq 1 20); do
    if curl -fsS http://127.0.0.1:8025/livez 2>/dev/null; then break; fi
    sleep 1
done
curl -fsS http://127.0.0.1:8025/livez || { echo "FAIL: /livez not reachable after service:start"; exit 1; }
echo "OK: /livez reachable after service:start"

echo "==> service:destroy mail"
pv service:destroy mail
test ! -f "$HOME/.pv/internal/bin/mailpit" || { echo "FAIL: mailpit binary not deleted after destroy"; exit 1; }
test ! -d "$HOME/.pv/services/mail/latest/data" || { echo "FAIL: data dir not deleted after destroy"; exit 1; }
echo "OK: binary and data removed"

echo "==> pv stop"
pv stop || true
trap - EXIT

echo "OK: Mail binary service lifecycle passed"
