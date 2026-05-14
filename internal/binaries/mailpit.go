package binaries

import (
	"fmt"
	"os"
	"runtime"
)

var Mailpit = Binary{
	Name:         "mailpit",
	DisplayName:  "Mailpit",
	NeedsExtract: true,
}

var mailpitPlatformNames = map[string]map[string]string{
	"darwin": {"arm64": "mac-arm64"},
}

// MailpitURL returns the pv artifact URL for Mailpit at the given version.
func MailpitURL(version string) (string, error) {
	if override := os.Getenv("PV_MAILPIT_URL_OVERRIDE"); override != "" {
		return override, nil
	}

	archMap, ok := mailpitPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for Mailpit: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for Mailpit: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/mailpit-%s-%s.tar.gz", platform, version), nil
}
