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
  sudo -E pv unlink e2e-s3-env >/dev/null 2>&1 || true
  sudo -E pv stop >/dev/null 2>&1 || true
  rm -rf "${ENVTEST_DIR:-}" 2>/dev/null || true
}
trap cleanup EXIT

# Install rustfs BEFORE pv link so ApplyPvYmlServicesStep finds it
# (PR 5 deleted the retroactive-bind path that used to fire on
# `pv rustfs:install`; binding now happens at link time via pv.yml).
echo "==> rustfs:install"
sudo -E pv rustfs:install || { echo "FAIL: pv rustfs:install failed"; exit 1; }

# Now link a Laravel project that declares rustfs + env template in pv.yml.
# pv link's ApplyPvYmlEnvStep will render the templates and write
# AWS_ENDPOINT into the project's .env.
ENVTEST_DIR=$(mktemp -d)
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0"}}' > "$ENVTEST_DIR/composer.json"
mkdir -p "$ENVTEST_DIR/public"
echo '<?php echo "test";' > "$ENVTEST_DIR/public/index.php"
echo "FILESYSTEM_DISK=local" > "$ENVTEST_DIR/.env"
cat > "$ENVTEST_DIR/pv.yml" << 'YMLEOF'
php: "8.4"
rustfs:
  env:
    AWS_ENDPOINT: "{{ .endpoint }}"
YMLEOF
sudo -E pv link "$ENVTEST_DIR" --name e2e-s3-env >/dev/null 2>&1 || { echo "FAIL: pv link for env test"; exit 1; }

echo "==> Verify rustfs binary exists"
test -x "$HOME/.pv/internal/bin/rustfs" || { echo "FAIL: rustfs binary not installed"; exit 1; }
echo "OK: rustfs binary at ~/.pv/internal/bin/rustfs"

echo "==> Verify daemon-status.json lists rustfs"
for i in $(seq 1 20); do
    if grep -q '"rustfs-latest"' "$HOME/.pv/daemon-status.json" 2>/dev/null; then break; fi
    sleep 1
done
grep -q '"rustfs-latest"' "$HOME/.pv/daemon-status.json" 2>/dev/null || {
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
nc -z 127.0.0.1 9000 || { echo "FAIL: port 9000 not reachable after rustfs:install"; exit 1; }
echo "OK: port 9000 reachable"

echo "==> Verify linked project .env got AWS_ENDPOINT via pv.yml env template"
# ApplyPvYmlEnvStep rendered rustfs.env.AWS_ENDPOINT against {{ .endpoint }}
# during `pv link`. Replaces the deleted retroactive-bind path that used to
# fire on `pv rustfs:install` for already-linked projects.
grep -q "AWS_ENDPOINT=http://127.0.0.1:9000" "$ENVTEST_DIR/.env" || {
    echo "FAIL: linked project .env should have AWS_ENDPOINT after pv link";
    echo "  actual .env contents:";
    cat "$ENVTEST_DIR/.env";
    exit 1;
}
echo "OK: linked project .env has AWS_ENDPOINT"

echo "==> rustfs:stop"
sudo -E pv rustfs:stop
sleep 2
if nc -z 127.0.0.1 9000 2>/dev/null; then
    echo "FAIL: port 9000 still answering after rustfs:stop"
    exit 1
fi
echo "OK: port 9000 silent after rustfs:stop"

echo "==> rustfs:start"
sudo -E pv rustfs:start
for i in $(seq 1 20); do
    if nc -z 127.0.0.1 9000 2>/dev/null; then break; fi
    sleep 1
done
nc -z 127.0.0.1 9000 || { echo "FAIL: port 9000 not reachable after rustfs:start"; exit 1; }
echo "OK: port 9000 reachable after rustfs:start"

echo "==> Verify s3:* alias is callable (s3:status)"
# Pointer-equality of RunE between alias and canonical is unit-tested in
# internal/commands/rustfs/register_test.go. This smoke just proves the
# built binary exposes the alias and can dispatch it without erroring.
sudo -E pv s3:status >/dev/null 2>&1 || {
    echo "FAIL: s3:status alias did not exit cleanly"
    exit 1
}
echo "OK: s3:* alias works"

echo "==> rustfs:uninstall --force"
sudo -E pv rustfs:uninstall --force
test ! -f "$HOME/.pv/internal/bin/rustfs" || { echo "FAIL: rustfs binary not deleted after uninstall"; exit 1; }
test ! -d "$HOME/.pv/services/s3/latest/data" || { echo "FAIL: data dir not deleted after uninstall"; exit 1; }
echo "OK: binary and data removed"

echo "==> pv stop"
sudo -E pv stop || true
trap - EXIT

echo "OK: S3 binary service lifecycle passed"
