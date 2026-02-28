package setup

import (
	"fmt"
	"os"
	"runtime"

	"github.com/prvious/pv/internal/config"
)

// CheckOS verifies we're running on macOS. Other platforms will be supported later.
func CheckOS() error {
	if runtime.GOOS != "darwin" {
		return fmt.Errorf("pv currently only supports macOS (detected: %s/%s)", runtime.GOOS, runtime.GOARCH)
	}
	return nil
}

// PlatformLabel returns a human-readable OS/arch label.
func PlatformLabel() string {
	return fmt.Sprintf("%s/%s", runtime.GOOS, runtime.GOARCH)
}

// IsAlreadyInstalled checks if pv has been installed before by looking for ~/.pv.
func IsAlreadyInstalled() bool {
	_, err := os.Stat(config.PvDir())
	return err == nil
}
