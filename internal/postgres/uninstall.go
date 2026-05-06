package postgres

import (
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Uninstall removes all on-disk state for a major: per-major services
// tree (which contains data/), binary tree, log file, state entry,
// version-tracking entry. Missing major is a no-op. Caller must stop
// the supervised process before calling.
func Uninstall(major string) error {
	// ServiceDataDir returns ~/.pv/services/postgres/<major>/data — wipe
	// the whole per-major dir, not just data, so we don't leave an empty
	// parent behind.
	if err := os.RemoveAll(filepath.Dir(config.ServiceDataDir("postgres", major))); err != nil {
		return err
	}
	if err := os.RemoveAll(config.PostgresVersionDir(major)); err != nil {
		return err
	}
	_ = os.Remove(config.PostgresLogPath(major))
	if err := RemoveMajor(major); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "postgres-"+major)
		_ = vs.Save()
	}
	return nil
}
