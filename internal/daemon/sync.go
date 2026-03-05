package daemon

import (
	"bytes"
	"fmt"
	"os"
)

// NeedsSync returns true if the plist on disk differs from what would be generated.
func NeedsSync(cfg PlistConfig) (bool, error) {
	expected, err := RenderPlist(cfg)
	if err != nil {
		return false, fmt.Errorf("cannot render plist for comparison: %w", err)
	}

	current, err := os.ReadFile(PlistPath())
	if err != nil {
		if os.IsNotExist(err) {
			return true, nil // No plist on disk, needs sync.
		}
		return false, fmt.Errorf("cannot read current plist: %w", err)
	}

	return !bytes.Equal(current, expected), nil
}

// SyncIfNeeded checks if the plist needs updating and reloads the daemon if so.
func SyncIfNeeded(cfg PlistConfig) error {
	needsSync, err := NeedsSync(cfg)
	if err != nil {
		return err
	}
	if !needsSync {
		return nil
	}

	if err := WritePlist(cfg); err != nil {
		return fmt.Errorf("cannot update plist: %w", err)
	}

	// If daemon is loaded, restart it to pick up changes.
	if IsLoaded() {
		if err := Restart(); err != nil {
			return fmt.Errorf("cannot restart daemon after plist update: %w", err)
		}
	}

	return nil
}
