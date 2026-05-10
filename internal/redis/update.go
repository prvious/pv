package redis

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Update redownloads the redis tarball and atomically replaces the
// binary tree. Data dir is untouched. If wanted=running before the
// update, restores wanted=running on success; otherwise leaves wanted
// as-is (user-driven).
func Update(client *http.Client) error {
	return UpdateProgress(client, nil)
}

// UpdateProgress is Update with a download progress callback.
func UpdateProgress(client *http.Client, progress binaries.ProgressFunc) error {
	if !IsInstalled() {
		return fmt.Errorf("redis is not installed")
	}

	// Snapshot prior wanted-state so we can restore it after a successful
	// update. A user who explicitly stopped redis before running
	// `redis:update` should NOT see it auto-start.
	prevWanted := WantedStopped
	if st, err := LoadState(); err == nil && st.Wanted != "" {
		prevWanted = st.Wanted
	}

	// Stop running daemon (if any) and wait for the TCP port to close
	// before swapping binaries.
	if prevWanted == WantedRunning {
		_ = SetWanted(WantedStopped)
		_ = WaitStopped(10 * time.Second)
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	dir := config.RedisDir()
	stagingDir := dir + ".new"
	os.RemoveAll(stagingDir)
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}

	archive := filepath.Join(config.PvDir(), "redis.tar.gz")
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
	oldDir := dir + ".old"
	os.RemoveAll(oldDir)
	if err := os.Rename(dir, oldDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename old: %w", err)
	}
	if err := os.Rename(stagingDir, dir); err != nil {
		if rollbackErr := os.Rename(oldDir, dir); rollbackErr != nil {
			return fmt.Errorf("rename new failed (%w); rollback also failed (%v); redis install dir is broken — manually mv %s %s",
				err, rollbackErr, oldDir, dir)
		}
		return fmt.Errorf("rename new: %w", err)
	}
	os.RemoveAll(oldDir)

	if err := chownToTarget(dir); err != nil {
		return fmt.Errorf("chown redis tree: %w", err)
	}

	// Re-probe + record version.
	if v, err := ProbeVersion(); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis", v)
			_ = vs.Save()
		}
	}

	// Restore prior wanted-state.
	return SetWanted(prevWanted)
}
