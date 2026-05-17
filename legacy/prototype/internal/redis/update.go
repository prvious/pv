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

func Update(client *http.Client, version string) error {
	return UpdateProgress(client, version, nil)
}

func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	if !IsInstalled(version) {
		return fmt.Errorf("redis-%s is not installed", version)
	}

	prevWanted := WantedStopped
	if st, err := LoadState(); err == nil {
		if vs, ok := st.Versions[version]; ok && vs.Wanted != "" {
			prevWanted = vs.Wanted
		}
	}

	if prevWanted == WantedRunning {
		_ = SetWanted(version, WantedStopped)
		_ = WaitStopped(version, 10*time.Second)
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	versionDir := config.RedisVersionDir(version)
	stagingDir := versionDir + ".new"
	os.RemoveAll(stagingDir)
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}

	archive := filepath.Join(config.PvDir(), "redis-"+version+".tar.gz")
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

	oldDir := versionDir + ".old"
	os.RemoveAll(oldDir)
	if err := os.Rename(versionDir, oldDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename old: %w", err)
	}
	if err := os.Rename(stagingDir, versionDir); err != nil {
		if rollbackErr := os.Rename(oldDir, versionDir); rollbackErr != nil {
			return fmt.Errorf("rename new failed (%w); rollback also failed (%v); redis %s install dir is broken",
				err, rollbackErr, version)
		}
		return fmt.Errorf("rename new: %w", err)
	}
	os.RemoveAll(oldDir)

	if err := chownToTarget(versionDir); err != nil {
		return fmt.Errorf("chown redis tree: %w", err)
	}

	if v, err := ProbeVersion(version); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis-"+version, v)
			_ = vs.Save()
		}
	}

	return SetWanted(version, prevWanted)
}
