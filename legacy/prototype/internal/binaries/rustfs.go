package binaries

import (
	"fmt"
	"os"
	"runtime"
)

var Rustfs = Binary{
	Name:         "rustfs",
	DisplayName:  "RustFS",
	NeedsExtract: true,
}

var rustfsPlatformNames = map[string]map[string]string{
	"darwin": {"arm64": "mac-arm64"},
}

// RustfsURL returns the pv artifact URL for RustFS at the given version.
func RustfsURL(version string) (string, error) {
	if override := os.Getenv("PV_RUSTFS_URL_OVERRIDE"); override != "" {
		return override, nil
	}

	archMap, ok := rustfsPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for RustFS: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for RustFS: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/rustfs-%s-%s.tar.gz", platform, version), nil
}
