package mailpit

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func Install(client *http.Client, version string) error {
	return InstallProgress(client, version, nil)
}

func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	if err := installArchive(client, version, progress); err != nil {
		return err
	}
	if err := os.MkdirAll(config.MailpitDataDir(version), 0o755); err != nil {
		return fmt.Errorf("create mailpit data dir: %w", err)
	}
	if err := recordInstalledVersion(version); err != nil {
		return err
	}
	return SetWanted(version, WantedRunning)
}

func installArchive(client *http.Client, version string, progress binaries.ProgressFunc) error {
	url, err := binaries.MailpitURL(version)
	if err != nil {
		return err
	}
	versionDir := config.MailpitVersionDir(version)
	stagingDir := versionDir + ".new"
	if err := os.RemoveAll(stagingDir); err != nil {
		return fmt.Errorf("clean staging: %w", err)
	}
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}
	archive := filepath.Join(config.PvDir(), "mailpit-"+version+".tar.gz")
	if err := binaries.DownloadProgress(client, url, archive, progress); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("download: %w", err)
	}
	if err := binaries.ExtractTarGzAll(archive, stagingDir); err != nil {
		os.RemoveAll(stagingDir)
		os.Remove(archive)
		return fmt.Errorf("extract: %w", err)
	}
	os.Remove(archive)
	binPath := filepath.Join(stagingDir, "bin", Binary().Name)
	st, err := os.Lstat(binPath)
	if err != nil {
		os.RemoveAll(stagingDir)
		return err
	}
	if !st.Mode().IsRegular() {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("mailpit archive binary is not a regular file: %s", binPath)
	}
	if err := binaries.MakeExecutable(binPath); err != nil {
		os.RemoveAll(stagingDir)
		return err
	}
	if err := swapVersionDir(versionDir, stagingDir); err != nil {
		os.RemoveAll(stagingDir)
		return err
	}
	return nil
}

func swapVersionDir(versionDir, stagingDir string) error {
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

func recordInstalledVersion(version string) error {
	recorded := version
	data, err := os.ReadFile(filepath.Join(config.MailpitVersionDir(version), "VERSION"))
	if err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("read mailpit artifact version: %w", err)
	}
	if err == nil {
		if trimmed := strings.TrimSpace(string(data)); trimmed != "" {
			recorded = trimmed
		}
	}
	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set("mailpit-"+version, recorded)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}
	return nil
}
