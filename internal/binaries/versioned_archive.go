package binaries

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
)

// VersionedArchiveInstall describes a pv-managed service archive install.
type VersionedArchiveInstall struct {
	ArtifactName string
	URL          string
	ArchivePath  string
	VersionDir   string
	BinaryName   string
	Progress     ProgressFunc
}

// InstallVersionedArchive downloads, validates, and atomically swaps a
// versioned service archive into place.
func InstallVersionedArchive(client *http.Client, install VersionedArchiveInstall) error {
	stagingDir := install.VersionDir + ".new"
	if err := os.RemoveAll(stagingDir); err != nil {
		return fmt.Errorf("clean staging: %w", err)
	}
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}
	if err := DownloadProgress(client, install.URL, install.ArchivePath, install.Progress); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("download: %w", err)
	}
	if err := ExtractTarGzAll(install.ArchivePath, stagingDir); err != nil {
		os.RemoveAll(stagingDir)
		os.Remove(install.ArchivePath)
		return fmt.Errorf("extract: %w", err)
	}
	os.Remove(install.ArchivePath)

	binPath := filepath.Join(stagingDir, "bin", install.BinaryName)
	st, err := os.Lstat(binPath)
	if err != nil {
		os.RemoveAll(stagingDir)
		return err
	}
	if !st.Mode().IsRegular() {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("%s archive binary is not a regular file: %s", install.ArtifactName, binPath)
	}
	if err := MakeExecutable(binPath); err != nil {
		os.RemoveAll(stagingDir)
		return err
	}
	if _, err := ReadArtifactVersion(stagingDir, install.ArtifactName); err != nil {
		os.RemoveAll(stagingDir)
		return err
	}
	if err := SwapVersionDir(install.VersionDir, stagingDir); err != nil {
		os.RemoveAll(stagingDir)
		return err
	}
	return nil
}

// SwapVersionDir atomically replaces versionDir with stagingDir and restores
// the previous versionDir if the final rename fails.
func SwapVersionDir(versionDir, stagingDir string) error {
	oldDir := versionDir + ".old"
	if err := os.RemoveAll(oldDir); err != nil {
		return fmt.Errorf("clean old version backup: %w", err)
	}

	hadCurrent := false
	if _, err := os.Lstat(versionDir); err == nil {
		hadCurrent = true
		if err := os.Rename(versionDir, oldDir); err != nil {
			return fmt.Errorf("move current version aside: %w", err)
		}
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("stat current version: %w", err)
	}

	if err := os.Rename(stagingDir, versionDir); err != nil {
		if hadCurrent {
			if restoreErr := os.Rename(oldDir, versionDir); restoreErr != nil {
				return fmt.Errorf("rename staging: %w; restore current version: %v", err, restoreErr)
			}
		}
		return fmt.Errorf("rename staging: %w", err)
	}

	if hadCurrent {
		if err := os.RemoveAll(oldDir); err != nil {
			return fmt.Errorf("remove old version backup: %w", err)
		}
	}
	return nil
}
