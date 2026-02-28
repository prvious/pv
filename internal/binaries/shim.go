package binaries

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// WriteComposerShim writes a shell shim at ~/.pv/bin/composer that runs composer.phar via the PHP CLI binary.
func WriteComposerShim() error {
	binDir := config.BinDir()
	php := filepath.Join(binDir, "php")
	composerPhar := filepath.Join(binDir, "composer.phar")
	content := fmt.Sprintf("#!/bin/sh\nexec \"%s\" \"%s\" \"$@\"\n", php, composerPhar)
	path := filepath.Join(binDir, "composer")
	if err := os.WriteFile(path, []byte(content), 0755); err != nil {
		return err
	}
	return nil
}

// WriteAllShims writes all shims (composer only — PHP is a real binary now).
func WriteAllShims() error {
	if err := WriteComposerShim(); err != nil {
		return fmt.Errorf("write composer shim: %w", err)
	}
	return nil
}
