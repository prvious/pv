package postgres

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Install downloads, extracts, initdb's, and registers a postgres major
// as "wanted=running". Idempotent: re-running on an already-installed
// major just re-emits conf overrides and re-marks state.
func Install(client *http.Client, major string) error {
	return InstallProgress(client, major, nil)
}

// InstallProgress is Install with a progress callback for the download phase.
func InstallProgress(client *http.Client, major string, progress binaries.ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	url, err := resolvePostgresURL(major)
	if err != nil {
		return err
	}

	versionDir := config.PostgresVersionDir(major)
	if !IsInstalled(major) {
		stagingDir := versionDir + ".new"
		os.RemoveAll(stagingDir)
		if err := os.MkdirAll(stagingDir, 0o755); err != nil {
			return fmt.Errorf("create staging: %w", err)
		}
		archive := filepath.Join(config.PostgresDir(), "postgres-"+major+".tar.gz")
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
	}

	if err := RunInitdb(major); err != nil {
		return err
	}
	if err := EnsureRuntime(major); err != nil {
		return err
	}

	if v, err := ProbeVersion(major); err == nil {
		vs, err := binaries.LoadVersions()
		if err == nil {
			vs.Set("postgres-"+major, v)
			_ = vs.Save()
		}
	}

	return SetWanted(major, "running")
}

// EnsureRuntime is the idempotent post-extract setup: refreshes
// postgresql.conf overrides, rewrites pg_hba.conf, and creates the /tmp
// socket dir. Safe to call repeatedly; called by Install on first install,
// and by the install command's idempotent short-circuit on re-install.
func EnsureRuntime(major string) error {
	if err := WriteOverrides(major); err != nil {
		return err
	}
	if err := RewriteHBA(major); err != nil {
		return err
	}
	if err := os.MkdirAll(socketDir(major), 0o755); err != nil {
		return fmt.Errorf("create socket dir: %w", err)
	}
	return nil
}

// resolvePostgresURL allows tests to redirect the download via env var.
// Production: returns the artifacts-release URL from binaries.PostgresURL.
func resolvePostgresURL(major string) (string, error) {
	if override := os.Getenv("PV_POSTGRES_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	return binaries.PostgresURL(major)
}
