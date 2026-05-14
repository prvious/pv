package binaries

import (
	"fmt"
	"runtime"
)

type Binary struct {
	Name         string
	DisplayName  string
	NeedsExtract bool
}

var Mago = Binary{
	Name:         "mago",
	DisplayName:  "Mago",
	NeedsExtract: true,
}

var Composer = Binary{
	Name:         "composer",
	DisplayName:  "Composer",
	NeedsExtract: false,
}

// Tools returns the standalone tool binaries (Mago, Composer).
// FrankenPHP and PHP CLI are managed by phpenv, not here.
func Tools() []Binary {
	return []Binary{Mago, Composer}
}

// DownloadURL returns the platform-specific download URL for a binary at the given version.
func DownloadURL(b Binary, version string) (string, error) {
	switch b.Name {
	case "mago":
		return magoURL(version)
	case "composer":
		return composerURL(), nil
	case "rustfs":
		return RustfsURL(version)
	case "mailpit":
		return MailpitURL(version)
	default:
		return "", fmt.Errorf("unknown binary: %s", b.Name)
	}
}

// ChecksumURL returns the checksum URL for a binary, or empty string if none available.
func ChecksumURL(b Binary, version string) (string, error) {
	switch b.Name {
	case "composer":
		return "https://getcomposer.org/download/latest-stable/composer.phar.sha256", nil
	default:
		return "", nil
	}
}

// LatestVersionURL returns the GitHub API URL for checking the latest release.
func LatestVersionURL(b Binary) string {
	switch b.Name {
	case "mago":
		return "https://api.github.com/repos/carthage-software/mago/releases/latest"
	case "rustfs":
		return "https://api.github.com/repos/rustfs/rustfs/releases?per_page=1"
	case "mailpit":
		return "https://api.github.com/repos/axllent/mailpit/releases/latest"
	default:
		return ""
	}
}

var magoPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "aarch64-apple-darwin",
		"amd64": "x86_64-apple-darwin",
	},
	"linux": {
		"amd64": "x86_64-unknown-linux-gnu",
		"arm64": "aarch64-unknown-linux-gnu",
	},
}

func magoArchiveName(version string) (string, error) {
	archMap, ok := magoPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for Mago: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for Mago: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("mago-%s-%s.tar.gz", version, platform), nil
}

func magoURL(version string) (string, error) {
	archive, err := magoArchiveName(version)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://github.com/carthage-software/mago/releases/download/%s/%s", version, archive), nil
}

func composerURL() string {
	return "https://getcomposer.org/download/latest-stable/composer.phar"
}
