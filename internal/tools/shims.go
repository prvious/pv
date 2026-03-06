package tools

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

const phpShimScript = `#!/bin/bash
# pv PHP version shim — auto-resolves PHP version per project.
set -euo pipefail

PV_PHP_DIR="%s"
PV_SETTINGS="%s"

# Read global default version from settings.
global_php() {
    if [ -f "$PV_SETTINGS" ]; then
        # Simple JSON parse for global_php field.
        grep -o '"global_php"[[:space:]]*:[[:space:]]*"[^"]*"' "$PV_SETTINGS" | \
            grep -o '"[^"]*"$' | tr -d '"' || true
    fi
}

# Walk up directories looking for .pv-php or composer.json.
resolve_version() {
    local dir="$PWD"
    while [ "$dir" != "/" ]; do
        # Check .pv-php file.
        if [ -f "$dir/.pv-php" ]; then
            cat "$dir/.pv-php" | tr -d '[:space:]'
            return
        fi
        # Check composer.json for PHP constraint (extract major.minor).
        if [ -f "$dir/composer.json" ]; then
            local constraint
            constraint=$(grep -o '"php"[[:space:]]*:[[:space:]]*"[^"]*"' "$dir/composer.json" | \
                grep -o '"[^"]*"$' | tr -d '"' || true)
            if [ -n "$constraint" ]; then
                # Extract the first major.minor version from the constraint.
                local ver
                ver=$(echo "$constraint" | grep -o '[0-9]\+\.[0-9]\+' | head -1)
                if [ -n "$ver" ] && [ -d "$PV_PHP_DIR/$ver" ]; then
                    echo "$ver"
                    return
                fi
            fi
        fi
        dir=$(dirname "$dir")
    done

    # Fall back to global default.
    global_php
}

VERSION=$(resolve_version)
if [ -z "$VERSION" ]; then
    echo "pv: no PHP version configured. Run: pv php:install <version>" >&2
    exit 1
fi

BINARY="$PV_PHP_DIR/$VERSION/php"
if [ ! -x "$BINARY" ]; then
    echo "pv: PHP $VERSION is not installed. Run: pv php:install $VERSION" >&2
    exit 1
fi

exec "$BINARY" "$@"
`

const composerShimScript = `#!/bin/bash
# pv Composer shim — isolates Composer home and cache under ~/.pv/composer.
set -euo pipefail

export COMPOSER_HOME="%s"
export COMPOSER_CACHE_DIR="%s"

PV_PHP_DIR="%s"
PV_SETTINGS="%s"
COMPOSER_PHAR="%s"

# Read global default version from settings.
global_php() {
    if [ -f "$PV_SETTINGS" ]; then
        grep -o '"global_php"[[:space:]]*:[[:space:]]*"[^"]*"' "$PV_SETTINGS" | \
            grep -o '"[^"]*"$' | tr -d '"' || true
    fi
}

# Walk up directories looking for .pv-php or composer.json.
resolve_version() {
    local dir="$PWD"
    while [ "$dir" != "/" ]; do
        if [ -f "$dir/.pv-php" ]; then
            cat "$dir/.pv-php" | tr -d '[:space:]'
            return
        fi
        if [ -f "$dir/composer.json" ]; then
            local constraint
            constraint=$(grep -o '"php"[[:space:]]*:[[:space:]]*"[^"]*"' "$dir/composer.json" | \
                grep -o '"[^"]*"$' | tr -d '"' || true)
            if [ -n "$constraint" ]; then
                local ver
                ver=$(echo "$constraint" | grep -o '[0-9]\+\.[0-9]\+' | head -1)
                if [ -n "$ver" ] && [ -d "$PV_PHP_DIR/$ver" ]; then
                    echo "$ver"
                    return
                fi
            fi
        fi
        dir=$(dirname "$dir")
    done
    global_php
}

VERSION=$(resolve_version)
if [ -z "$VERSION" ]; then
    echo "pv: no PHP version configured. Run: pv php:install <version>" >&2
    exit 1
fi

PHP_BINARY="$PV_PHP_DIR/$VERSION/php"
if [ ! -x "$PHP_BINARY" ]; then
    echo "pv: PHP $VERSION is not installed. Run: pv php:install $VERSION" >&2
    exit 1
fi

exec "$PHP_BINARY" "$COMPOSER_PHAR" "$@"
`

// writePhpShim writes the PHP version-resolving shim to ~/.pv/bin/php.
func writePhpShim() error {
	phpDir := config.PhpDir()
	settingsPath := config.SettingsPath()
	binDir := config.BinDir()

	shimPath := filepath.Join(binDir, "php")
	content := fmt.Sprintf(phpShimScript, phpDir, settingsPath)
	if err := os.WriteFile(shimPath, []byte(content), 0755); err != nil {
		return fmt.Errorf("cannot write php shim: %w", err)
	}
	return nil
}

// writeComposerShim writes the Composer shim to ~/.pv/bin/composer.
func writeComposerShim() error {
	binDir := config.BinDir()

	shimPath := filepath.Join(binDir, "composer")
	content := fmt.Sprintf(composerShimScript,
		config.ComposerDir(),
		config.ComposerCacheDir(),
		config.PhpDir(),
		config.SettingsPath(),
		config.ComposerPharPath(),
	)
	if err := os.WriteFile(shimPath, []byte(content), 0755); err != nil {
		return fmt.Errorf("cannot write composer shim: %w", err)
	}
	return nil
}
