package binaries

import (
	"fmt"
	"runtime"
)

var Rustfs = Binary{
	Name:         "rustfs",
	DisplayName:  "RustFS",
	NeedsExtract: true,
}

// rustfsPlatformNames maps GOOS/GOARCH to the platform suffix RustFS uses in
// its release asset filenames. Linux releases publish -gnu (glibc) and -musl
// variants; pv ships the -gnu variant for the broadest compatibility with
// typical developer machines.
var rustfsPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "macos-aarch64",
		"amd64": "macos-x86_64",
	},
	"linux": {
		"amd64": "linux-x86_64-gnu",
		"arm64": "linux-aarch64-gnu",
	},
}

func rustfsArchiveName() (string, error) {
	archMap, ok := rustfsPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for RustFS: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for RustFS: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("rustfs-%s-latest.zip", platform), nil
}

func rustfsURL(version string) (string, error) {
	archive, err := rustfsArchiveName()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://github.com/rustfs/rustfs/releases/download/%s/%s", version, archive), nil
}
