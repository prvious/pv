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
echo "==> settings.json"
cat ~/.pv/config/settings.json 2>/dev/null || echo "(no settings.json)"
echo ""
echo "==> versions.json"
cat ~/.pv/data/versions.json 2>/dev/null || echo "(no versions.json)"
echo ""
echo "==> PHP dirs"
ls -laR ~/.pv/php/ 2>/dev/null || echo "(no php dir)"
echo ""
echo "==> bin dir"
ls -la ~/.pv/bin/ 2>/dev/null || echo "(no bin dir)"
