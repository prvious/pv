package tools

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

const phpShimScript = `#!/bin/bash
# pv PHP version shim — delegates version resolution to pv binary.
set -euo pipefail

PV_PHP_DIR="%s"
PV_BIN="%s"

VERSION=$("$PV_BIN" php:current)
if [ -z "$VERSION" ]; then
    echo "pv: no PHP version configured. Run: pv php:install [version]" >&2
    exit 1
fi

BINARY="$PV_PHP_DIR/$VERSION/php"
if [ ! -x "$BINARY" ]; then
    echo "pv: PHP $VERSION is not installed. Run: pv php:install $VERSION" >&2
    exit 1
fi

exec "$BINARY" "$@"
`

const colimaShimScript = `#!/bin/sh
# pv Colima shim — ensures Lima (limactl) is on PATH.
export PATH="%s:$PATH"
exec "%s" "$@"
`

// writeColimaShim writes the Colima wrapper shim to ~/.pv/bin/colima.
func writeColimaShim() error {
	limaBinDir := config.LimaBinDir()
	colimaPath := config.ColimaPath()
	binDir := config.BinDir()

	shimPath := filepath.Join(binDir, "colima")
	content := fmt.Sprintf(colimaShimScript, limaBinDir, colimaPath)
	if err := os.WriteFile(shimPath, []byte(content), 0755); err != nil {
		return fmt.Errorf("cannot write colima shim: %w", err)
	}
	return nil
}

// writePhpShim writes the PHP version-resolving shim to ~/.pv/bin/php.
func writePhpShim() error {
	phpDir := config.PhpDir()
	binDir := config.BinDir()

	pvBin, err := os.Executable()
	if err != nil {
		return fmt.Errorf("cannot find pv binary: %w", err)
	}
	if resolved, err := filepath.EvalSymlinks(pvBin); err == nil {
		pvBin = resolved
	}

	shimPath := filepath.Join(binDir, "php")
	content := fmt.Sprintf(phpShimScript, phpDir, pvBin)
	if err := os.WriteFile(shimPath, []byte(content), 0755); err != nil {
		return fmt.Errorf("cannot write php shim: %w", err)
	}
	return nil
}
