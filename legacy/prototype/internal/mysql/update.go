package mysql

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update redownloads the mysql tarball for a version and atomically
// replaces the binary tree. Data dir is untouched (auto.cnf present →
// RunInitdb is a no-op). If wanted=running before the update, restores
// wanted=running on success; otherwise leaves wanted as-is (user-driven).
func Update(client *http.Client, version string) error {
	return UpdateProgress(client, version, nil)
}

// UpdateProgress is Update with a download progress callback.
func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if !IsInstalled(version) {
		return fmt.Errorf("mysql %s is not installed", version)
	}

	// Snapshot prior wanted-state so we can restore it after a successful
	// update. A user who had explicitly stopped the version before running
	// `mysql:update` should NOT see it auto-start.
	prevWanted := WantedStopped
	if st, err := LoadState(); err == nil {
		if v, ok := st.Versions[version]; ok {
			prevWanted = v.Wanted
		}
	}

	// Stop the running daemon (if any) and wait for the TCP port to close
	// before swapping binaries — InnoDB needs to flush.
	if prevWanted == WantedRunning {
		_ = SetWanted(version, WantedStopped)
		_ = WaitStopped(version, 30*time.Second)
	}

	url, err := resolveMysqlURL(version)
	if err != nil {
		return err
	}

	versionDir := config.MysqlVersionDir(version)
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
			return fmt.Errorf("rename new failed (%w); rollback also failed (%v); mysql %s install dir is broken — manually mv %s %s",
				err, rollbackErr, version, oldDir, versionDir)
		}
		return fmt.Errorf("rename new: %w", err)
	}
	os.RemoveAll(oldDir)

	// Hand new binary tree to SUDO_USER if running as root.
	if err := chownToTarget(versionDir); err != nil {
		return fmt.Errorf("chown mysql tree: %w", err)
	}

	// Re-probe + record version (patch level may have moved).
	if v, err := ProbeVersion(version); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("mysql-"+version, v)
			_ = vs.Save()
		}
	}

	// Restore prior wanted-state. If it was running, bring it back; if it
	// was stopped, leave it stopped.
	return SetWanted(version, prevWanted)
}
