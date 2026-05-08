#!/usr/bin/env bash
# No set -e: we want to dump as much as possible even if some commands fail.

echo "==> caddy.log (last 100 lines)"
tail -100 ~/.pv/logs/caddy.log 2>/dev/null || echo "(no caddy.log)"
echo ""
echo "==> caddy-8.3.log"
cat ~/.pv/logs/caddy-8.3.log 2>/dev/null || echo "(no caddy-8.3.log)"
echo ""
echo "==> Main Caddyfile"
cat ~/.pv/config/Caddyfile 2>/dev/null || echo "(no Caddyfile)"
echo ""
echo "==> php-8.3.Caddyfile"
cat ~/.pv/config/php-8.3.Caddyfile 2>/dev/null || echo "(no php-8.3.Caddyfile)"
echo ""
echo "==> sites/ dir"
ls -la ~/.pv/config/sites/ 2>/dev/null || echo "(no sites dir)"
echo ""
echo "==> sites-8.3/ dir"
ls -la ~/.pv/config/sites-8.3/ 2>/dev/null || echo "(no sites-8.3 dir)"
echo ""
echo "==> Site config contents (sites/)"
for f in ~/.pv/config/sites/*.caddy; do
  echo "--- $f ---"
  cat "$f" 2>/dev/null || true
done
echo ""
echo "==> Site config contents (sites-8.3/)"
for f in ~/.pv/config/sites-8.3/*.caddy; do
  echo "--- $f ---"
  cat "$f" 2>/dev/null || true
done
echo ""
echo "==> registry.json"
cat ~/.pv/data/registry.json 2>/dev/null || echo "(no registry.json)"
echo ""
echo "==> pv.yml"
cat ~/.pv/pv.yml 2>/dev/null || echo "(no pv.yml)"
echo ""
echo "==> versions.json"
cat ~/.pv/data/versions.json 2>/dev/null || echo "(no versions.json)"
echo ""
echo "==> PHP dirs"
ls -laR ~/.pv/php/ 2>/dev/null || echo "(no php dir)"
echo ""
echo "==> bin dir"
ls -la ~/.pv/bin/ 2>/dev/null || echo "(no bin dir)"
echo ""
echo "==> composer dir"
ls -laR ~/.pv/composer/ 2>/dev/null || echo "(no composer dir)"
echo ""
echo "==> composer.phar"
ls -la ~/.pv/internal/bin/composer.phar 2>/dev/null || echo "(no composer.phar)"
echo ""
echo "==> composer shim contents"
cat ~/.pv/bin/composer 2>/dev/null || echo "(no composer shim)"
echo ""
echo "==> services dir"
ls -laR ~/.pv/services/ 2>/dev/null || echo "(no services dir)"
echo ""
echo "==> colima binary"
ls -la ~/.pv/bin/colima 2>/dev/null || echo "(no colima binary)"
echo ""
echo "==> colima version"
~/.pv/bin/colima version 2>/dev/null || echo "(colima version failed)"
echo ""
echo "==> colima status"
~/.pv/bin/colima status --profile pv 2>/dev/null || echo "(colima not running)"
echo ""
echo "==> pv service:list"
pv service:list 2>&1 || echo "(pv service:list failed)"
echo ""
echo "==> state.json"
cat ~/.pv/data/state.json 2>/dev/null || echo "(no state.json)"
echo ""
echo "==> daemon-status.json"
cat ~/.pv/daemon-status.json 2>/dev/null || echo "(no daemon-status.json)"
echo ""
echo "==> mysql logs"
for f in ~/.pv/logs/mysql-*.log; do
  [ -e "$f" ] || continue
  echo "--- $f ---"
  tail -100 "$f" 2>/dev/null || echo "(unreadable)"
done
echo ""
echo "==> postgres logs"
for f in ~/.pv/logs/postgres-*.log; do
  [ -e "$f" ] || continue
  echo "--- $f ---"
  tail -100 "$f" 2>/dev/null || echo "(unreadable)"
done
echo ""
echo "==> /tmp pv e2e logs"
for f in /tmp/pv-mysql-e2e.log /tmp/pv-postgres-e2e.log /tmp/pv-mail-e2e.log /tmp/pv-s3-e2e.log; do
  [ -e "$f" ] || continue
  echo "--- $f ---"
  tail -100 "$f" 2>/dev/null || echo "(unreadable)"
done
echo ""
echo "==> mysql data dirs"
ls -la ~/.pv/data/mysql/ 2>/dev/null || echo "(no mysql data dir)"
for d in ~/.pv/data/mysql/*/; do
  [ -d "$d" ] || continue
  echo "--- contents of $d ---"
  ls -la "$d" 2>/dev/null | head -20
done
echo ""
echo "==> mysql binary trees"
ls -la ~/.pv/mysql/ 2>/dev/null || echo "(no mysql binary dir)"
echo ""
echo "==> /tmp pv-mysql sockets/pids"
ls -la /tmp/pv-mysql-* 2>/dev/null || echo "(no /tmp/pv-mysql files)"
