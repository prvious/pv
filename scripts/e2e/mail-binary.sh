#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> Phase: Mail binary service (Mailpit) lifecycle"

# e2e tests use foreground mode with sudo (previous phases leave root-owned
# config dirs; only root can clean and regenerate them).
sudo -E pv start >/tmp/pv-mail-e2e.log 2>&1 &
START_PID=$!
sleep 8

cleanup() {
  sudo -E pv unlink e2e-mail-env >/dev/null 2>&1 || true
  sudo -E pv stop >/dev/null 2>&1 || true
  rm -rf "${ENVTEST_DIR:-}" 2>/dev/null || true
}
trap cleanup EXIT

# Create a minimal linked Laravel project so we can assert .env injection.
ENVTEST_DIR=$(mktemp -d)
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0"}}' > "$ENVTEST_DIR/composer.json"
mkdir -p "$ENVTEST_DIR/public"
echo '<?php echo "test";' > "$ENVTEST_DIR/public/index.php"
echo "MAIL_MAILER=log" > "$ENVTEST_DIR/.env"
sudo -E pv link "$ENVTEST_DIR" --name e2e-mail-env >/dev/null 2>&1 || { echo "FAIL: pv link for env test"; exit 1; }

echo "==> service:add mail"
sudo -E pv service:add mail || { echo "FAIL: pv service:add mail failed"; exit 1; }

echo "==> Verify mailpit binary exists"
test -x "$HOME/.pv/internal/bin/mailpit" || { echo "FAIL: mailpit binary not installed"; exit 1; }
echo "OK: mailpit binary at ~/.pv/internal/bin/mailpit"

echo "==> Verify daemon-status.json lists mailpit"
for i in $(seq 1 20); do
    if grep -q '"mailpit"' "$HOME/.pv/daemon-status.json" 2>/dev/null; then break; fi
    sleep 1
done
grep -q '"mailpit"' "$HOME/.pv/daemon-status.json" 2>/dev/null || {
    echo "FAIL: daemon-status.json does not contain mailpit entry";
    cat "$HOME/.pv/daemon-status.json" 2>/dev/null || echo "(file missing)";
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

echo "==> Verify linked project .env got MAIL_MAILER=smtp"
grep -q "MAIL_MAILER=smtp" "$ENVTEST_DIR/.env" || {
    echo "FAIL: linked project .env should have MAIL_MAILER=smtp after service:add mail";
    echo "  actual .env contents:";
    cat "$ENVTEST_DIR/.env";
    exit 1;
}
echo "OK: linked project .env has MAIL_MAILER=smtp"

echo "==> service:stop mail"
sudo -E pv service:stop mail
sleep 2
if curl -fsS http://127.0.0.1:8025/livez 2>/dev/null; then
    echo "FAIL: /livez still answering after service:stop"
    exit 1
fi
echo "OK: /livez silent after service:stop"

echo "==> service:start mail"
sudo -E pv service:start mail
for i in $(seq 1 20); do
    if curl -fsS http://127.0.0.1:8025/livez 2>/dev/null; then break; fi
    sleep 1
done
curl -fsS http://127.0.0.1:8025/livez || { echo "FAIL: /livez not reachable after service:start"; exit 1; }
echo "OK: /livez reachable after service:start"

echo "==> service:destroy mail"
sudo -E pv service:destroy mail
test ! -f "$HOME/.pv/internal/bin/mailpit" || { echo "FAIL: mailpit binary not deleted after destroy"; exit 1; }
test ! -d "$HOME/.pv/services/mail/latest/data" || { echo "FAIL: data dir not deleted after destroy"; exit 1; }
echo "OK: binary and data removed"

echo "==> pv stop"
sudo -E pv stop || true
trap - EXIT

echo "OK: Mail binary service lifecycle passed"
