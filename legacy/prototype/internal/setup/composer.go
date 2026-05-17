package setup

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// MigrateComposerConfig checks for existing ~/.composer config files and
// copies them into ~/.pv/composer/ so tokens and repository configs are preserved.
func MigrateComposerConfig() {
	home, err := os.UserHomeDir()
	if err != nil {
		return
	}

	oldComposerDir := filepath.Join(home, ".composer")
	filesToMigrate := []string{"auth.json", "config.json"}

	for _, name := range filesToMigrate {
		src := filepath.Join(oldComposerDir, name)
		dst := filepath.Join(config.ComposerDir(), name)

		if _, err := os.Stat(src); os.IsNotExist(err) {
			continue
		}
		if _, err := os.Stat(dst); err == nil {
			// Already exists in pv, skip.
			continue
		}

		data, err := os.ReadFile(src)
		if err != nil {
			continue
		}

		if err := os.WriteFile(dst, data, 0600); err != nil {
			fmt.Printf("  Warning: could not copy %s: %v\n", name, err)
			continue
		}
		fmt.Printf("  ✓ Imported %s from ~/.composer\n", name)
	}
}
