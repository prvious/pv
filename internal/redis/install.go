package redis

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

func Install(client *http.Client, version string) error {
	return InstallProgress(client, version, nil)
}

func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	versionDir := config.RedisVersionDir(version)
	if !IsInstalled(version) {
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
		os.RemoveAll(versionDir)
		if err := os.Rename(stagingDir, versionDir); err != nil {
			os.RemoveAll(stagingDir)
			return fmt.Errorf("rename staging: %w", err)
		}
		if err := chownToTarget(versionDir); err != nil {
			return fmt.Errorf("chown redis tree: %w", err)
		}
	}

	dataDir := config.RedisDataDirV(version)
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return fmt.Errorf("create redis data dir: %w", err)
	}
	if err := chownToTarget(dataDir); err != nil {
		return fmt.Errorf("chown redis data dir: %w", err)
	}

	if v, err := ProbeVersion(version); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis-"+version, v)
			_ = vs.Save()
		}
	}

	return SetWanted(version, WantedRunning)
}

func resolveRedisURL() (string, error) {
	if override := os.Getenv("PV_REDIS_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	return binaries.RedisURL()
}
