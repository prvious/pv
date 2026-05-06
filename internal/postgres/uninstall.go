package postgres

import (
	"os"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
)

// Uninstall removes all on-disk state for a major: data dir, binary tree,
// log file, state entry, version-tracking entry. Missing major is a no-op.
// Caller must stop the supervised process before calling.
func Uninstall(major string) error {
	if err := os.RemoveAll(config.ServiceDataDir("postgres", major)); err != nil {
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
