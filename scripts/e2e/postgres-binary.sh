#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> Phase: PostgreSQL native-binary lifecycle (PG 17 + 18)"

# Start pv in foreground so the supervisor reconciles postgres state.
sudo -E pv start >/tmp/pv-postgres-e2e.log 2>&1 &
START_PID=$!
sleep 8

cleanup() {
  sudo -E pv unlink e2e-postgres-env >/dev/null 2>&1 || true
  sudo -E pv postgres:uninstall 17 --force >/dev/null 2>&1 || true
  sudo -E pv postgres:uninstall 18 --force >/dev/null 2>&1 || true
  sudo -E pv stop >/dev/null 2>&1 || true
  rm -rf "${ENVTEST_DIR:-}" 2>/dev/null || true
}
trap cleanup EXIT

# Pre-link a Laravel project so the postgres binding flow is exercised.
ENVTEST_DIR=$(mktemp -d)
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0"}}' > "$ENVTEST_DIR/composer.json"
mkdir -p "$ENVTEST_DIR/public"
echo '<?php echo "test";' > "$ENVTEST_DIR/public/index.php"
echo "DB_CONNECTION=pgsql" > "$ENVTEST_DIR/.env"
sudo -E pv link "$ENVTEST_DIR" --name e2e-postgres-env >/dev/null 2>&1 || { echo "FAIL: pv link"; exit 1; }

echo "==> postgres:install 17"
sudo -E pv postgres:install 17 || { echo "FAIL: postgres:install 17"; exit 1; }

echo "==> postgres:install 18"
sudo -E pv postgres:install 18 || { echo "FAIL: postgres:install 18"; exit 1; }

echo "==> Verify both binary trees exist"
test -x "$HOME/.pv/postgres/17/bin/postgres" || { echo "FAIL: PG 17 binary missing"; exit 1; }
test -x "$HOME/.pv/postgres/18/bin/postgres" || { echo "FAIL: PG 18 binary missing"; exit 1; }
echo "OK: both binary trees present"

echo "==> Verify both ports accept connections"
for i in $(seq 1 30); do
    if nc -z 127.0.0.1 54017 2>/dev/null && nc -z 127.0.0.1 54018 2>/dev/null; then break; fi
    sleep 1
done
nc -z 127.0.0.1 54017 || { echo "FAIL: port 54017 (PG17) not reachable"; exit 1; }
nc -z 127.0.0.1 54018 || { echo "FAIL: port 54018 (PG18) not reachable"; exit 1; }
echo "OK: 54017 + 54018 both reachable"

echo "==> Verify daemon-status.json lists both supervised processes"
grep -q '"postgres-17"' "$HOME/.pv/daemon-status.json" || { echo "FAIL: postgres-17 missing from daemon-status.json"; exit 1; }
grep -q '"postgres-18"' "$HOME/.pv/daemon-status.json" || { echo "FAIL: postgres-18 missing from daemon-status.json"; exit 1; }
echo "OK: daemon-status.json advertises both"

echo "==> psql sanity (PG 17)"
PG17_VER=$("$HOME/.pv/postgres/17/bin/psql" -h 127.0.0.1 -p 54017 -U postgres -tAc "SELECT version();" | head -1)
echo "  $PG17_VER"
echo "$PG17_VER" | grep -q "PostgreSQL 17" || { echo "FAIL: PG 17 didn't report 17.x"; exit 1; }

echo "==> psql sanity (PG 18)"
PG18_VER=$("$HOME/.pv/postgres/18/bin/psql" -h 127.0.0.1 -p 54018 -U postgres -tAc "SELECT version();" | head -1)
echo "  $PG18_VER"
echo "$PG18_VER" | grep -q "PostgreSQL 18" || { echo "FAIL: PG 18 didn't report 18.x"; exit 1; }

echo "==> Cross-major isolation: db created on 18 must not be visible on 17"
"$HOME/.pv/postgres/18/bin/psql" -h 127.0.0.1 -p 54018 -U postgres -c "CREATE DATABASE e2e_pg18_only;" >/dev/null
SEEN_ON_17=$("$HOME/.pv/postgres/17/bin/psql" -h 127.0.0.1 -p 54017 -U postgres -tAc "SELECT 1 FROM pg_database WHERE datname='e2e_pg18_only';" | head -1)
SEEN_ON_18=$("$HOME/.pv/postgres/18/bin/psql" -h 127.0.0.1 -p 54018 -U postgres -tAc "SELECT 1 FROM pg_database WHERE datname='e2e_pg18_only';" | head -1)
[ -z "$SEEN_ON_17" ] || { echo "FAIL: e2e_pg18_only leaked to PG 17"; exit 1; }
[ "$SEEN_ON_18" = "1" ] || { echo "FAIL: e2e_pg18_only not visible on PG 18"; exit 1; }
echo "OK: cross-major isolation confirmed"

echo "==> Verify linked project got DB_PORT for the highest-installed major (18 → 54018)"
grep -q "DB_PORT=54018" "$ENVTEST_DIR/.env" || {
    echo "FAIL: linked project .env should have DB_PORT=54018";
    echo "  actual .env contents:";
    cat "$ENVTEST_DIR/.env";
    exit 1;
}
echo "OK: linked project .env has DB_PORT=54018"

echo "==> postgres:stop 17 — only PG 18 should still serve"
sudo -E pv postgres:stop 17
for i in $(seq 1 10); do
    if ! nc -z 127.0.0.1 54017 2>/dev/null; then break; fi
    sleep 1
done
if nc -z 127.0.0.1 54017 2>/dev/null; then echo "FAIL: 54017 still answering after stop"; exit 1; fi
nc -z 127.0.0.1 54018 || { echo "FAIL: 54018 should still be up"; exit 1; }
echo "OK: PG 17 stopped, PG 18 still serving"

echo "==> postgres:start 17 — both should serve again"
sudo -E pv postgres:start 17
for i in $(seq 1 30); do
    if nc -z 127.0.0.1 54017 2>/dev/null; then break; fi
    sleep 1
done
nc -z 127.0.0.1 54017 || { echo "FAIL: 54017 not reachable after start"; exit 1; }
echo "OK: PG 17 back online"

echo "==> postgres:uninstall 17 --force"
sudo -E pv postgres:uninstall 17 --force
test ! -d "$HOME/.pv/postgres/17" || { echo "FAIL: PG 17 binary tree not removed"; exit 1; }
test ! -d "$HOME/.pv/services/postgres/17" || { echo "FAIL: PG 17 data dir not removed"; exit 1; }
echo "OK: PG 17 fully removed"

echo "==> postgres:uninstall 18 --force"
sudo -E pv postgres:uninstall 18 --force
test ! -d "$HOME/.pv/postgres/18" || { echo "FAIL: PG 18 binary tree not removed"; exit 1; }
echo "OK: PG 18 fully removed"

echo "==> postgres:list reports nothing"
sudo -E pv postgres:list 2>&1 | grep -q "No PostgreSQL majors installed" || { echo "FAIL: list should be empty"; exit 1; }

echo "==> pv stop"
sudo -E pv stop || true
trap - EXIT

echo "OK: PostgreSQL native-binary lifecycle passed"
