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

var FrankenPHP = Binary{
	Name:         "frankenphp",
	DisplayName:  "FrankenPHP",
	NeedsExtract: false,
}

var Mago = Binary{
	Name:         "mago",
	DisplayName:  "Mago",
	NeedsExtract: true,
}

var PHP = Binary{
	Name:         "php",
	DisplayName:  "PHP CLI",
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
	case "frankenphp":
		return frankenphpURL(version)
	case "mago":
		return magoURL(version)
	case "composer":
		return composerURL(), nil
	case "php":
		return phpURL(version)
	default:
		return "", fmt.Errorf("unknown binary: %s", b.Name)
	}
}

// ChecksumURL returns the checksum URL for a binary, or empty string if none available.
func ChecksumURL(b Binary, version string) (string, error) {
	switch b.Name {
	case "frankenphp":
		// FrankenPHP releases don't include per-file checksum assets.
		return "", nil
	case "composer":
		return "https://getcomposer.org/download/latest-stable/composer.phar.sha256", nil
	default:
		return "", nil
	}
}

// LatestVersionURL returns the GitHub API URL for checking the latest release.
func LatestVersionURL(b Binary) string {
	switch b.Name {
	case "frankenphp":
		return "https://api.github.com/repos/dunglas/frankenphp/releases/latest"
	case "mago":
		return "https://api.github.com/repos/carthage-software/mago/releases/latest"
	case "php":
		return "" // PHP version comes from FrankenPHP, not GitHub
	default:
		return ""
	}
}

var frankenphpPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "frankenphp-mac-arm64",
		"amd64": "frankenphp-mac-x86_64",
	},
	"linux": {
		"amd64": "frankenphp-linux-x86_64",
		"arm64": "frankenphp-linux-aarch64",
	},
}

func frankenphpBinaryName() (string, error) {
	archMap, ok := frankenphpPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for FrankenPHP: %s", runtime.GOOS)
	}
	name, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for FrankenPHP: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return name, nil
}

func frankenphpURL(version string) (string, error) {
	name, err := frankenphpBinaryName()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://github.com/dunglas/frankenphp/releases/download/v%s/%s", version, name), nil
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

var phpArchNames = map[string]string{
	"arm64": "aarch64",
	"amd64": "x86_64",
}

var phpOSNames = map[string]string{
	"darwin": "macos",
	"linux":  "linux",
}

func phpURL(version string) (string, error) {
	arch, ok := phpArchNames[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for PHP CLI: %s", runtime.GOARCH)
	}
	osName, ok := phpOSNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for PHP CLI: %s", runtime.GOOS)
	}
	return fmt.Sprintf("https://dl.static-php.dev/static-php-cli/common/php-%s-cli-%s-%s.tar.gz", version, osName, arch), nil
}
