#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> Phase: Redis native-binary lifecycle"

# Start pv in foreground so the supervisor reconciles redis state.
sudo -E pv start >/tmp/pv-redis-e2e.log 2>&1 &
START_PID=$!
sleep 8

cleanup() {
  sudo -E pv unlink e2e-redis-env >/dev/null 2>&1 || true
  sudo -E pv redis:uninstall --force >/dev/null 2>&1 || true
  sudo -E pv stop >/dev/null 2>&1 || true
  rm -rf "${ENVTEST_DIR:-}" 2>/dev/null || true
}
trap cleanup EXIT

# Pre-link a Laravel project so the redis auto-bind path is exercised.
ENVTEST_DIR=$(mktemp -d)
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0"}}' > "$ENVTEST_DIR/composer.json"
mkdir -p "$ENVTEST_DIR/public"
echo '<?php echo "test";' > "$ENVTEST_DIR/public/index.php"
echo "APP_NAME=test" > "$ENVTEST_DIR/.env"
sudo -E pv link "$ENVTEST_DIR" --name e2e-redis-env >/dev/null 2>&1 || { echo "FAIL: pv link"; exit 1; }

echo "==> redis:install"
sudo -E pv redis:install || { echo "FAIL: redis:install"; exit 1; }

echo "==> Verify binary tree exists"
test -x "$HOME/.pv/redis/redis-server" || { echo "FAIL: redis-server binary missing"; exit 1; }
test -x "$HOME/.pv/redis/redis-cli" || { echo "FAIL: redis-cli binary missing"; exit 1; }
echo "OK: redis binary tree present"

echo "==> Wait for port 6379 to accept connections"
wait_for_tcp 127.0.0.1 6379 30 || { echo "FAIL: 6379 not reachable"; exit 1; }
echo "OK: 6379 reachable"

echo "==> Verify daemon-status.json lists redis"
grep -q '"redis"' "$HOME/.pv/daemon-status.json" || { echo "FAIL: redis missing from daemon-status.json"; exit 1; }
echo "OK: daemon-status.json advertises redis"

echo "==> redis-cli PING"
PING=$("$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 PING | tr -d '[:space:]')
[ "$PING" = "PONG" ] || { echo "FAIL: PING returned '$PING', want 'PONG'"; exit 1; }
echo "OK: PING returned PONG"

echo "==> redis-cli SET/GET roundtrip"
"$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 SET pv_e2e_key "hello-world" >/dev/null
GOT=$("$HOME/.pv/redis/redis-cli" -h 127.0.0.1 -p 6379 GET pv_e2e_key)
[ "$GOT" = "hello-world" ] || { echo "FAIL: GET returned '$GOT', want 'hello-world'"; exit 1; }
echo "OK: SET/GET roundtrip"

echo "==> Verify pre-linked project got REDIS_HOST=127.0.0.1 (auto-bind retroactive)"
grep -q "REDIS_HOST=127.0.0.1" "$ENVTEST_DIR/.env" || {
    echo "FAIL: linked project .env should have REDIS_HOST=127.0.0.1";
    echo "  actual .env contents:";
    cat "$ENVTEST_DIR/.env";
    exit 1;
}
grep -q "REDIS_PORT=6379" "$ENVTEST_DIR/.env" || { echo "FAIL: missing REDIS_PORT=6379"; exit 1; }
grep -q "REDIS_PASSWORD=null" "$ENVTEST_DIR/.env" || { echo "FAIL: missing REDIS_PASSWORD=null"; exit 1; }
echo "OK: linked project .env has REDIS_*"

echo "==> redis:list shows the row"
LIST=$(sudo -E pv redis:list 2>&1)
echo "$LIST" | strip_ansi | grep -q "6379" || { echo "FAIL: list missing port 6379"; echo "$LIST"; exit 1; }
echo "OK: redis:list shows the row"

echo "==> redis:stop"
sudo -E pv redis:stop
for i in $(seq 1 10); do
    if ! nc -z 127.0.0.1 6379 2>/dev/null; then break; fi
    sleep 1
done
if nc -z 127.0.0.1 6379 2>/dev/null; then echo "FAIL: 6379 still answering after stop"; exit 1; fi
echo "OK: redis stopped"

echo "==> redis:start"
sudo -E pv redis:start
wait_for_tcp 127.0.0.1 6379 30 || { echo "FAIL: 6379 not reachable after start"; exit 1; }
echo "OK: redis back online"

echo "==> redis:uninstall --force"
sudo -E pv redis:uninstall --force
test ! -d "$HOME/.pv/redis" || { echo "FAIL: redis binary tree not removed"; exit 1; }
test ! -d "$HOME/.pv/data/redis" || { echo "FAIL: redis data dir not removed"; exit 1; }
echo "OK: redis fully removed"

echo "==> daemon-status.json no longer lists redis"
sleep 2
grep -q '"redis"' "$HOME/.pv/daemon-status.json" && { echo "FAIL: redis still in daemon-status after uninstall"; exit 1; } || true
echo "OK: redis cleared from daemon-status"

echo "==> pv stop"
sudo -E pv stop || true
trap - EXIT

echo "OK: Redis native-binary lifecycle passed"
