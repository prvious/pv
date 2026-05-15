package mysql

import (
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

// Uninstall removes on-disk state for a mysql version. With force=false:
// removes binary tree, log file, state entry, version-tracking entry; the
// data dir at ~/.pv/data/mysql/<version>/ is preserved. With force=true:
// also removes the data dir.
//
// Caller's responsibility to handle the running daemon — Uninstall sets
// wanted=stopped and waits up to 30s for the TCP port to close before
// removing files (mysqld's InnoDB shutdown can take a moment to flush).
// Missing version is a no-op.
func Uninstall(version string, force bool) error {
	if isInstalledOnDisk(version) {
		_ = SetWanted(version, WantedStopped)
		_ = WaitStopped(version, 30*time.Second)
	}

	if err := os.RemoveAll(config.MysqlVersionDir(version)); err != nil {
		return err
	}
	_ = os.Remove(config.MysqlLogPath(version))
	if force {
		if err := os.RemoveAll(config.MysqlDataDir(version)); err != nil {
			return err
		}
	}
	if err := RemoveVersion(version); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "mysql-"+version)
		_ = vs.Save()
	}
	if reg, err := registry.Load(); err == nil {
		reg.UnbindMysqlVersion(version)
		_ = reg.Save()
	}
	return nil
}

// isInstalledOnDisk is a cheap pre-check used by Uninstall to skip the
// 30s wait when there's nothing on disk.
func isInstalledOnDisk(version string) bool {
	_, err := os.Stat(config.MysqlBinDir(version))
	return err == nil
}
