#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

echo "==> Phase: MySQL native-binary lifecycle (8.4 + 9.7)"

# Start pv in foreground so the supervisor reconciles mysql state.
sudo -E pv start >/tmp/pv-mysql-e2e.log 2>&1 &
START_PID=$!
sleep 8

cleanup() {
  sudo -E pv unlink e2e-mysql-env >/dev/null 2>&1 || true
  sudo -E pv mysql:uninstall 8.4 --force >/dev/null 2>&1 || true
  sudo -E pv mysql:uninstall 9.7 --force >/dev/null 2>&1 || true
  sudo -E pv stop >/dev/null 2>&1 || true
  rm -rf "${ENVTEST_DIR:-}" 2>/dev/null || true
}
trap cleanup EXIT

# Install mysql 8.4 BEFORE pv link so ApplyPvYmlServicesStep finds it
# (PR 5 deleted the retroactive-bind path; binding now happens at link time
# via pv.yml).
echo "==> mysql:install 8.4"
sudo -E pv mysql:install 8.4 || { echo "FAIL: mysql:install 8.4"; exit 1; }

echo "==> mysql:install 9.7"
sudo -E pv mysql:install 9.7 || { echo "FAIL: mysql:install 9.7"; exit 1; }

# Now link a Laravel project that declares mysql 8.4 + env template in pv.yml.
# pv link's ApplyPvYmlEnvStep will render the templates and write DB_PORT
# into the project's .env (the path that replaced the deleted retroactive
# bind on `pv mysql:install`).
ENVTEST_DIR=$(mktemp -d)
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0"}}' > "$ENVTEST_DIR/composer.json"
mkdir -p "$ENVTEST_DIR/public"
echo '<?php echo "test";' > "$ENVTEST_DIR/public/index.php"
echo "DB_CONNECTION=mysql" > "$ENVTEST_DIR/.env"
cat > "$ENVTEST_DIR/pv.yml" << 'YMLEOF'
php: "8.4"
mysql:
  version: "8.4"
  env:
    DB_CONNECTION: mysql
    DB_HOST: "{{ .host }}"
    DB_PORT: "{{ .port }}"
YMLEOF
sudo -E pv link "$ENVTEST_DIR" --name e2e-mysql-env >/dev/null 2>&1 || { echo "FAIL: pv link"; exit 1; }

echo "==> Verify both binary trees exist"
test -x "$HOME/.pv/mysql/8.4/bin/mysqld" || { echo "FAIL: mysql 8.4 binary missing"; exit 1; }
test -x "$HOME/.pv/mysql/9.7/bin/mysqld" || { echo "FAIL: mysql 9.7 binary missing"; exit 1; }
echo "OK: both binary trees present"

echo "==> Wait for both ports to accept connections"
wait_for_tcp 127.0.0.1 33084 60 || { echo "FAIL: 33084 (mysql 8.4) not reachable"; exit 1; }
wait_for_tcp 127.0.0.1 33097 60 || { echo "FAIL: 33097 (mysql 9.7) not reachable"; exit 1; }
echo "OK: 33084 + 33097 both reachable"

echo "==> Verify daemon-status.json lists both supervised processes"
grep -q '"mysql-8.4"' "$HOME/.pv/daemon-status.json" || { echo "FAIL: mysql-8.4 missing from daemon-status.json"; exit 1; }
grep -q '"mysql-9.7"' "$HOME/.pv/daemon-status.json" || { echo "FAIL: mysql-9.7 missing from daemon-status.json"; exit 1; }
echo "OK: daemon-status.json advertises both"

echo "==> Connect via bundled mysql client over unix socket (8.4)"
MY84_VER=$("$HOME/.pv/mysql/8.4/bin/mysql" --socket=/tmp/pv-mysql-8.4.sock -u root -e "SELECT VERSION();" -sN | head -1)
echo "  $MY84_VER"
echo "$MY84_VER" | grep -q "^8\.4" || { echo "FAIL: mysql 8.4 didn't report 8.4.x, got: $MY84_VER"; exit 1; }

echo "==> Connect via bundled mysql client over unix socket (9.7)"
MY97_VER=$("$HOME/.pv/mysql/9.7/bin/mysql" --socket=/tmp/pv-mysql-9.7.sock -u root -e "SELECT VERSION();" -sN | head -1)
echo "  $MY97_VER"
echo "$MY97_VER" | grep -q "^9\.7" || { echo "FAIL: mysql 9.7 didn't report 9.7.x, got: $MY97_VER"; exit 1; }

echo "==> Cross-version isolation: db created on 8.4 must not be visible on 9.7"
"$HOME/.pv/mysql/8.4/bin/mysql" --socket=/tmp/pv-mysql-8.4.sock -u root -e "CREATE DATABASE e2e_my84_only;" >/dev/null
SEEN_ON_84=$("$HOME/.pv/mysql/8.4/bin/mysql" --socket=/tmp/pv-mysql-8.4.sock -u root -sN -e "SHOW DATABASES LIKE 'e2e_my84_only';" | head -1)
SEEN_ON_97=$("$HOME/.pv/mysql/9.7/bin/mysql" --socket=/tmp/pv-mysql-9.7.sock -u root -sN -e "SHOW DATABASES LIKE 'e2e_my84_only';" | head -1)
[ "$SEEN_ON_84" = "e2e_my84_only" ] || { echo "FAIL: e2e_my84_only not visible on mysql 8.4"; exit 1; }
[ -z "$SEEN_ON_97" ] || { echo "FAIL: e2e_my84_only leaked to mysql 9.7"; exit 1; }
echo "OK: cross-version isolation confirmed"

echo "==> Verify linked project got DB_PORT via pv.yml env template (mysql 8.4 → 33084)"
# ApplyPvYmlEnvStep renders mysql.env.DB_PORT against {{ .port }} during
# `pv link`. This replaces the deleted retroactive-bind path that used to
# fire on `pv mysql:install` for already-linked projects.
grep -q "DB_PORT=33084" "$ENVTEST_DIR/.env" || {
    echo "FAIL: linked project .env should have DB_PORT=33084";
    echo "  actual .env contents:";
    cat "$ENVTEST_DIR/.env";
    exit 1;
}
echo "OK: linked project .env has DB_PORT=33084"

echo "==> mysql:list shows both rows"
LIST=$(sudo -E pv mysql:list 2>&1)
echo "$LIST" | strip_ansi | grep -q "8\.4" || { echo "FAIL: list missing 8.4"; echo "$LIST"; exit 1; }
echo "$LIST" | strip_ansi | grep -q "9\.7" || { echo "FAIL: list missing 9.7"; echo "$LIST"; exit 1; }
echo "OK: mysql:list shows both"

echo "==> mysql:stop 8.4 — only 9.7 should still serve"
sudo -E pv mysql:stop 8.4
for i in $(seq 1 10); do
    if ! nc -z 127.0.0.1 33084 2>/dev/null; then break; fi
    sleep 1
done
if nc -z 127.0.0.1 33084 2>/dev/null; then echo "FAIL: 33084 still answering after stop"; exit 1; fi
nc -z 127.0.0.1 33097 || { echo "FAIL: 33097 should still be up"; exit 1; }
echo "OK: mysql 8.4 stopped, mysql 9.7 still serving"

echo "==> mysql:start 8.4 — both should serve again"
sudo -E pv mysql:start 8.4
wait_for_tcp 127.0.0.1 33084 30 || { echo "FAIL: 33084 not reachable after start"; exit 1; }
echo "OK: mysql 8.4 back online"

echo "==> mysql:uninstall 8.4 --force"
sudo -E pv mysql:uninstall 8.4 --force
test ! -d "$HOME/.pv/mysql/8.4" || { echo "FAIL: mysql 8.4 binary tree not removed"; exit 1; }
test ! -d "$HOME/.pv/data/mysql/8.4" || { echo "FAIL: mysql 8.4 data dir not removed"; exit 1; }
echo "OK: mysql 8.4 fully removed"

echo "==> mysql:list shows only 9.7 left"
LIST=$(sudo -E pv mysql:list 2>&1)
echo "$LIST" | strip_ansi | grep -q "9\.7" || { echo "FAIL: 9.7 missing from list after 8.4 uninstall"; exit 1; }
echo "$LIST" | strip_ansi | grep -q "8\.4" && { echo "FAIL: 8.4 still in list after uninstall"; exit 1; }
echo "OK: only 9.7 remains"

echo "==> mysql:uninstall 9.7 --force"
sudo -E pv mysql:uninstall 9.7 --force
test ! -d "$HOME/.pv/mysql/9.7" || { echo "FAIL: mysql 9.7 binary tree not removed"; exit 1; }
echo "OK: mysql 9.7 fully removed"

echo "==> pv stop"
sudo -E pv stop || true
trap - EXIT

echo "OK: MySQL native-binary lifecycle passed"
