package binaries

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// Verbose controls whether install functions print progress details.
var Verbose bool

func logf(format string, args ...any) {
	if Verbose {
		fmt.Printf(format, args...)
	}
}

// InstallBinary downloads and installs a single binary at the given version.
// If progress is non-nil, it is called during download with bytes written and total.
func InstallBinary(client *http.Client, b Binary, version string) error {
	return InstallBinaryProgress(client, b, version, nil)
}

// InstallBinaryProgress downloads and installs a single binary with optional progress.
func InstallBinaryProgress(client *http.Client, b Binary, version string, progress ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	binDir := config.BinDir()

	url, err := DownloadURL(b, version)
	if err != nil {
		return err
	}

	switch b.Name {
	case "frankenphp":
		return installFrankenPHP(client, url, b, version, binDir, progress)
	case "mago":
		return installMago(client, url, b, binDir, progress)
	case "composer":
		return installComposer(client, url, b, version, binDir, progress)
	case "php":
		return installPHP(client, url, b, binDir, progress)
	default:
		return fmt.Errorf("unknown binary: %s", b.Name)
	}
}

func installFrankenPHP(client *http.Client, url string, b Binary, version string, binDir string, progress ProgressFunc) error {
	destPath := filepath.Join(binDir, "frankenphp")

	logf("  Downloading %s...\n", b.DisplayName)
	if err := DownloadProgress(client, url, destPath, progress); err != nil {
		return err
	}

	checksumURL, err := ChecksumURL(b, version)
	if err != nil {
		return err
	}
	if checksumURL != "" {
		logf("  Verifying checksum...\n")
		expected, err := FetchChecksum(client, checksumURL)
		if err != nil {
			return err
		}
		if err := VerifyChecksum(destPath, expected); err != nil {
			os.Remove(destPath)
			return err
		}
	}

	return MakeExecutable(destPath)
}

func installMago(client *http.Client, url string, b Binary, binDir string, progress ProgressFunc) error {
	archivePath := filepath.Join(binDir, "mago.tar.gz")
	destPath := filepath.Join(binDir, "mago")

	logf("  Downloading %s...\n", b.DisplayName)
	if err := DownloadProgress(client, url, archivePath, progress); err != nil {
		return err
	}

	logf("  Extracting...\n")
	if err := ExtractTarGz(archivePath, destPath, "mago"); err != nil {
		return err
	}

	os.Remove(archivePath)
	return MakeExecutable(destPath)
}

func installPHP(client *http.Client, url string, b Binary, binDir string, progress ProgressFunc) error {
	archivePath := filepath.Join(binDir, "php.tar.gz")
	destPath := filepath.Join(binDir, "php")

	logf("  Downloading %s...\n", b.DisplayName)
	if err := DownloadProgress(client, url, archivePath, progress); err != nil {
		return err
	}

	logf("  Extracting...\n")
	if err := ExtractTarGz(archivePath, destPath, "php"); err != nil {
		return err
	}

	os.Remove(archivePath)
	return MakeExecutable(destPath)
}

func installComposer(client *http.Client, url string, b Binary, version string, binDir string, progress ProgressFunc) error {
	destPath := config.ComposerPharPath()

	logf("  Downloading %s...\n", b.DisplayName)
	if err := DownloadProgress(client, url, destPath, progress); err != nil {
		return err
	}

	checksumURL, err := ChecksumURL(b, version)
	if err != nil {
		return err
	}
	if checksumURL != "" {
		logf("  Verifying checksum...\n")
		expected, err := FetchChecksum(client, checksumURL)
		if err != nil {
			return err
		}
		if err := VerifyChecksum(destPath, expected); err != nil {
			os.Remove(destPath)
			return err
		}
	}

	return nil
}
