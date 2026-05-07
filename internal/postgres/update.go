package postgres

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update redownloads the postgres tarball for a major and re-applies
// conf overrides. Data dir is untouched. Marks wanted=running on success.
// Caller must have stopped the supervised process before calling.
func Update(client *http.Client, major string) error {
	return UpdateProgress(client, major, nil)
}

// UpdateProgress is Update with a download progress callback.
func UpdateProgress(client *http.Client, major string, progress binaries.ProgressFunc) error {
	if !IsInstalled(major) {
		return fmt.Errorf("postgres %s is not installed", major)
	}

	url, err := resolvePostgresURL(major)
	if err != nil {
		return err
	}

	versionDir := config.PostgresVersionDir(major)
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

	// Two-phase swap (NOT atomic — two os.Rename calls). If the second
	// rename fails we attempt a best-effort restore; if THAT also fails
	// the user is in a half-broken state and must know about it.
	oldDir := versionDir + ".old"
	os.RemoveAll(oldDir)
	if err := os.Rename(versionDir, oldDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename old: %w", err)
	}
	if err := os.Rename(stagingDir, versionDir); err != nil {
		if rollbackErr := os.Rename(oldDir, versionDir); rollbackErr != nil {
			return fmt.Errorf("rename new failed (%w); rollback also failed (%v); postgres %s install dir is broken — manually mv %s %s",
				err, rollbackErr, major, oldDir, versionDir)
		}
		return fmt.Errorf("rename new: %w", err)
	}
	os.RemoveAll(oldDir)

	if err := WriteOverrides(major); err != nil {
		return err
	}
	if err := RewriteHBA(major); err != nil {
		return err
	}

	if v, err := ProbeVersion(major); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("postgres-"+major, v)
			_ = vs.Save()
		}
	}

	return SetWanted(major, WantedRunning)
}
