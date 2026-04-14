package binaries

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

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

	url, err := DownloadURL(b, version)
	if err != nil {
		return err
	}

	switch b.Name {
	case "composer":
		return installComposer(client, url, b, version, progress)
	case "mago":
		return installMago(client, url, progress)
	case "rustfs":
		return installRustfs(client, url, progress)
	default:
		return fmt.Errorf("unknown binary: %s", b.Name)
	}
}

func installMago(client *http.Client, url string, progress ProgressFunc) error {
	internalBin := config.InternalBinDir()
	archivePath := filepath.Join(internalBin, "mago.tar.gz")
	destPath := filepath.Join(internalBin, "mago")

	if err := DownloadProgress(client, url, archivePath, progress); err != nil {
		return err
	}

	if err := ExtractTarGz(archivePath, destPath, "mago"); err != nil {
		return err
	}

	os.Remove(archivePath)
	return MakeExecutable(destPath)
}

func installRustfs(client *http.Client, url string, progress ProgressFunc) error {
	internalBin := config.InternalBinDir()
	archivePath := filepath.Join(internalBin, "rustfs.zip")
	destPath := filepath.Join(internalBin, "rustfs")

	if err := DownloadProgress(client, url, archivePath, progress); err != nil {
		return err
	}
	if err := ExtractZip(archivePath, destPath, "rustfs"); err != nil {
		return err
	}
	os.Remove(archivePath)
	return MakeExecutable(destPath)
}

func installComposer(client *http.Client, url string, b Binary, version string, progress ProgressFunc) error {
	destPath := config.ComposerPharPath()

	if err := DownloadProgress(client, url, destPath, progress); err != nil {
		return err
	}

	checksumURL, err := ChecksumURL(b, version)
	if err != nil {
		return err
	}
	if checksumURL != "" {
		expected, err := FetchChecksum(client, checksumURL)
		if err != nil {
			return err
		}
		if err := VerifyChecksum(destPath, expected); err != nil {
			os.Remove(destPath)
			return err
		}
	}

	return os.Chmod(destPath, 0755)
}
