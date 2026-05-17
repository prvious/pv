package mysql

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Install downloads, extracts, inits, and registers a mysql version as
// "wanted=running". Idempotent: re-running on an already-installed
// version is a no-op for files (skips download/extract/init) and just
// re-records the version + wanted=running.
func Install(client *http.Client, version string) error {
	return InstallProgress(client, version, nil)
}

// InstallProgress is Install with a progress callback for the download phase.
func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	url, err := resolveMysqlURL(version)
	if err != nil {
		return err
	}

	versionDir := config.MysqlVersionDir(version)
	if !IsInstalled(version) {
		stagingDir := versionDir + ".new"
		os.RemoveAll(stagingDir)
		if err := os.MkdirAll(stagingDir, 0o755); err != nil {
			return fmt.Errorf("create staging: %w", err)
		}
		archive := filepath.Join(config.MysqlDir(), "mysql-"+version+".tar.gz")
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
		os.RemoveAll(versionDir)
		if err := os.Rename(stagingDir, versionDir); err != nil {
			os.RemoveAll(stagingDir)
			return fmt.Errorf("rename staging: %w", err)
		}
		// When pv runs as root (e.g. `sudo pv start` to bind :443), hand
		// the binary tree to the SUDO_USER so the dropped mysqld process
		// can read its own dylibs / share files.
		if err := chownToTarget(versionDir); err != nil {
			return fmt.Errorf("chown mysql tree: %w", err)
		}
	}

	// Init is gated by auto.cnf — skipped if already initialized.
	if err := RunInitdb(version); err != nil {
		return err
	}

	if v, err := ProbeVersion(version); err == nil {
		vs, err := binaries.LoadVersions()
		if err == nil {
			vs.Set("mysql-"+version, v)
			_ = vs.Save()
		}
	}

	return SetWanted(version, WantedRunning)
}

// resolveMysqlURL allows tests to redirect the download via env var.
// Production: returns the artifacts-release URL from binaries.MysqlURL.
func resolveMysqlURL(version string) (string, error) {
	if override := os.Getenv("PV_MYSQL_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	return binaries.MysqlURL(version)
}
