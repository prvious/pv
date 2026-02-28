package phpenv

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
    echo "pv: no PHP version configured. Run: pv php install <version>" >&2
    exit 1
fi

BINARY="$PV_PHP_DIR/$VERSION/php"
if [ ! -x "$BINARY" ]; then
    echo "pv: PHP $VERSION is not installed. Run: pv php install $VERSION" >&2
    exit 1
fi

exec "$BINARY" "$@"
`

// WriteShims creates the php and frankenphp shim scripts in ~/.pv/bin/.
func WriteShims() error {
	phpDir := config.PhpDir()
	settingsPath := config.SettingsPath()
	binDir := config.BinDir()

	// Write PHP shim.
	phpShim := filepath.Join(binDir, "php")
	content := fmt.Sprintf(phpShimScript, phpDir, settingsPath)
	if err := os.WriteFile(phpShim, []byte(content), 0755); err != nil {
		return fmt.Errorf("cannot write php shim: %w", err)
	}

	return nil
}
