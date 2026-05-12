package redis

import (
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func Uninstall(version string, force bool) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	if isInstalledOnDisk(version) {
		_ = SetWanted(version, WantedStopped)
		_ = WaitStopped(version, 10*time.Second)
	}

	reg, err := registry.Load()
	if err != nil {
		return err
	}
	reg.UnbindRedisVersion(version)
	if err := reg.Save(); err != nil {
		return err
	}

	if err := RemoveVersion(version); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "redis-"+version)
		_ = vs.Save()
	}

	if err := os.RemoveAll(config.RedisVersionDir(version)); err != nil {
		return err
	}
	_ = os.Remove(config.RedisLogPathV(version))
	if force {
		if err := os.RemoveAll(config.RedisDataDirV(version)); err != nil {
			return err
		}
	}
	return nil
}

func isInstalledOnDisk(version string) bool {
	_, err := os.Stat(config.RedisVersionDir(version))
	return err == nil
}
