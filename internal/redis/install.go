package redis

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Install downloads, extracts, and registers redis as wanted=running.
// Idempotent: re-running on an already-installed redis is a no-op for
// files (skips download/extract) and just re-records wanted=running.
//
// Note there is NO init step (redis has no `--initialize-insecure`
// equivalent — RDB persistence is created on first save by redis-server
// itself, no schema bootstrap is needed).
func Install(client *http.Client) error {
	return InstallProgress(client, nil)
}

// InstallProgress is Install with a progress callback for the download phase.
func InstallProgress(client *http.Client, progress binaries.ProgressFunc) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}

	url, err := resolveRedisURL()
	if err != nil {
		return err
	}

	dir := config.RedisDir()
	if !IsInstalled() {
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
		os.RemoveAll(dir)
		if err := os.Rename(stagingDir, dir); err != nil {
			os.RemoveAll(stagingDir)
			return fmt.Errorf("rename staging: %w", err)
		}
		// When pv runs as root (sudo pv start), hand the binary tree to
		// SUDO_USER so the dropped redis-server process can read it.
		if err := chownToTarget(dir); err != nil {
			return fmt.Errorf("chown redis tree: %w", err)
		}
	}

	// Create + chown the data dir to SUDO_USER so the dropped
	// redis-server can write dump.rdb. EnsureDirs deliberately doesn't
	// create this — see internal/config/paths.go's comment about why.
	if err := os.MkdirAll(config.RedisDataDir(), 0o755); err != nil {
		return fmt.Errorf("create redis data dir: %w", err)
	}
	if err := chownToTarget(config.RedisDataDir()); err != nil {
		return fmt.Errorf("chown redis data dir: %w", err)
	}

	// Probe + record version. Best-effort: a probe failure shouldn't
	// fail the install (the binary is already on disk and runnable; the
	// version record is diagnostic).
	if v, err := ProbeVersion(); err == nil {
		if vs, err := binaries.LoadVersions(); err == nil {
			vs.Set("redis", v)
			_ = vs.Save()
		}
	}

	return SetWanted(WantedRunning)
}

// resolveRedisURL allows tests to redirect the download via env var.
// Production: returns the artifacts-release URL from binaries.RedisURL.
func resolveRedisURL() (string, error) {
	if override := os.Getenv("PV_REDIS_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	return binaries.RedisURL()
}
