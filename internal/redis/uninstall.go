package redis

import (
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// Uninstall removes on-disk state for redis. With force=false: removes
// binary tree, log file, state entry, and version-tracking entry; the
// data dir at ~/.pv/data/redis/ is preserved. With force=true: also
// removes the data dir.
//
// Caller's responsibility to handle the running daemon — Uninstall sets
// wanted=stopped and waits up to 10s for the TCP port to close before
// removing files.
//
// Ordering note: state/versions/registry saves happen BEFORE file
// removal. Each of those Save calls routes through config.EnsureDirs,
// which (unlike mysql/postgres where EnsureDirs only creates the parent
// dir) recreates RedisDir and RedisDataDir directly — so removing the
// dirs first and then saving state would leave behind empty dirs.
func Uninstall(force bool) error {
	if isInstalledOnDisk() {
		v, _ := ProbeVersion()
		if v == "" {
			v = "unknown"
		}
		_ = SetWanted(v, WantedStopped)
		_ = WaitStopped(v, 10*time.Second)
	}

	// Update bookkeeping first — these saves call EnsureDirs internally
	// and would otherwise recreate the dirs we're about to remove.
	if err := RemoveState(); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "redis")
		_ = vs.Save()
	}
	if reg, err := registry.Load(); err == nil {
		// UnbindService("redis") already exists in registry.go and clears
		// Services.Redis on every project — we don't need a redis-specific
		// helper because redis is single-version (mysql/postgres needed
		// version-aware helpers because their bindings carry a version).
		reg.UnbindService("redis")
		_ = reg.Save()
	}

	// File removal last so the bookkeeping saves above don't recreate the
	// dirs via EnsureDirs.
	if err := os.RemoveAll(config.RedisDir()); err != nil {
		return err
	}
	_ = os.Remove(config.RedisLogPath())
	if force {
		if err := os.RemoveAll(config.RedisDataDir()); err != nil {
			return err
		}
	}
	return nil
}

// isInstalledOnDisk is a cheap pre-check used by Uninstall to skip the
// 10s wait when there's nothing on disk.
func isInstalledOnDisk() bool {
	_, err := os.Stat(config.RedisDir())
	return err == nil
}
