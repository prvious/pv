package binaries

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// InstallBinary downloads and installs a single binary at the given version.
func InstallBinary(client *http.Client, b Binary, version string) error {
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
		return installFrankenPHP(client, url, b, version, binDir)
	case "mago":
		return installMago(client, url, b, binDir)
	case "composer":
		return installComposer(client, url, b, version, binDir)
	case "php":
		return installPHP(client, url, b, binDir)
	default:
		return fmt.Errorf("unknown binary: %s", b.Name)
	}
}

func installFrankenPHP(client *http.Client, url string, b Binary, version string, binDir string) error {
	destPath := filepath.Join(binDir, "frankenphp")

	fmt.Printf("  Downloading %s...\n", b.DisplayName)
	if err := Download(client, url, destPath); err != nil {
		return err
	}

	checksumURL, err := ChecksumURL(b, version)
	if err != nil {
		return err
	}
	if checksumURL != "" {
		fmt.Printf("  Verifying checksum...\n")
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

func installMago(client *http.Client, url string, b Binary, binDir string) error {
	archivePath := filepath.Join(binDir, "mago.tar.gz")
	destPath := filepath.Join(binDir, "mago")

	fmt.Printf("  Downloading %s...\n", b.DisplayName)
	if err := Download(client, url, archivePath); err != nil {
		return err
	}

	fmt.Printf("  Extracting...\n")
	if err := ExtractTarGz(archivePath, destPath, "mago"); err != nil {
		return err
	}

	os.Remove(archivePath)
	return MakeExecutable(destPath)
}

func installPHP(client *http.Client, url string, b Binary, binDir string) error {
	archivePath := filepath.Join(binDir, "php.tar.gz")
	destPath := filepath.Join(binDir, "php")

	fmt.Printf("  Downloading %s...\n", b.DisplayName)
	if err := Download(client, url, archivePath); err != nil {
		return err
	}

	fmt.Printf("  Extracting...\n")
	if err := ExtractTarGz(archivePath, destPath, "php"); err != nil {
		return err
	}

	os.Remove(archivePath)
	return MakeExecutable(destPath)
}

func installComposer(client *http.Client, url string, b Binary, version string, binDir string) error {
	destPath := filepath.Join(binDir, "composer")

	fmt.Printf("  Downloading %s...\n", b.DisplayName)
	if err := Download(client, url, destPath); err != nil {
		return err
	}

	checksumURL, err := ChecksumURL(b, version)
	if err != nil {
		return err
	}
	if checksumURL != "" {
		fmt.Printf("  Verifying checksum...\n")
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
