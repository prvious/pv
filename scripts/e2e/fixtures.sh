#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/helpers.sh"

# 1. Static site
mkdir -p /tmp/e2e-static
echo '<h1>static works</h1>' > /tmp/e2e-static/index.html

# 2. PHP site (resolves to global 8.4 via composer constraint)
mkdir -p /tmp/e2e-php/public
echo '{"require":{"php":"^8.0"}}' > /tmp/e2e-php/composer.json
cat > /tmp/e2e-php/public/index.php << 'PHPEOF'
<?php
ignore_user_abort(true);
$handler = static function () { echo "php works"; };
for (;;) {
    if (!\frankenphp_handle_request($handler)) break;
}
PHPEOF

# 3. Laravel site (resolves to global 8.4)
mkdir -p /tmp/e2e-laravel/public
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0"}}' > /tmp/e2e-laravel/composer.json
cat > /tmp/e2e-laravel/public/index.php << 'PHPEOF'
<?php
ignore_user_abort(true);
$handler = static function () { echo "laravel works"; };
for (;;) {
    if (!\frankenphp_handle_request($handler)) break;
}
PHPEOF

# 4. Laravel Octane site (resolves to global 8.4, detected via octane + worker file)
mkdir -p /tmp/e2e-octane/public
echo '{"require":{"php":"^8.2","laravel/framework":"^11.0","laravel/octane":"^2.0"}}' > /tmp/e2e-octane/composer.json
cat > /tmp/e2e-octane/public/frankenphp-worker.php << 'PHPEOF'
<?php
ignore_user_abort(true);
$handler = static function () { echo "octane works"; };
for (;;) {
    if (!\frankenphp_handle_request($handler)) break;
}
PHPEOF

# 5. PHP 8.3 site (multi-version via .pv-php override)
mkdir -p /tmp/e2e-php83/public
echo '{"require":{"php":"^8.0"}}' > /tmp/e2e-php83/composer.json
echo '8.3' > /tmp/e2e-php83/.pv-php
cat > /tmp/e2e-php83/public/index.php << 'PHPEOF'
<?php
ignore_user_abort(true);
$handler = static function () { echo "php83 works"; };
for (;;) {
    if (!\frankenphp_handle_request($handler)) break;
}
PHPEOF

# 6. Dynamic site (linked while server running, resolves to global 8.4)
mkdir -p /tmp/e2e-dynamic/public
echo '{"require":{"php":"^8.0"}}' > /tmp/e2e-dynamic/composer.json
cat > /tmp/e2e-dynamic/public/index.php << 'PHPEOF'
<?php
ignore_user_abort(true);
$handler = static function () { echo "dynamic works"; };
for (;;) {
    if (!\frankenphp_handle_request($handler)) break;
}
PHPEOF

echo "OK: 6 fixtures created"
