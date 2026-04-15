package binaries

import (
	"fmt"
	"runtime"
)

var Mailpit = Binary{
	Name:         "mailpit",
	DisplayName:  "Mailpit",
	NeedsExtract: true,
}

var mailpitPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "darwin-arm64",
		"amd64": "darwin-amd64",
	},
	"linux": {
		"amd64": "linux-amd64",
		"arm64": "linux-arm64",
	},
}

func mailpitArchiveName() (string, error) {
	archMap, ok := mailpitPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for Mailpit: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for Mailpit: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("mailpit-%s.tar.gz", platform), nil
}

func mailpitURL(version string) (string, error) {
	archive, err := mailpitArchiveName()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://github.com/axllent/mailpit/releases/download/%s/%s", version, archive), nil
}
