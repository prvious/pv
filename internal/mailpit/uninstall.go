package mailpit

import (
	"fmt"
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
	_ = SetWanted(version, WantedStopped)
	_ = WaitStopped(version, 30*time.Second)

	binPath, err := BinaryPath(version)
	if err != nil {
		return err
	}
	if err := os.Remove(binPath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("remove mailpit binary: %w", err)
	}
	if logPath, err := LogPath(version); err == nil {
		_ = os.Remove(logPath)
	}
	if err := RemoveVersion(version); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, Binary().Name)
		_ = vs.Save()
	}
	if force {
		if err := os.RemoveAll(config.ServiceDataDir(ServiceKey(), version)); err != nil {
			return fmt.Errorf("cannot delete data: %w", err)
		}
	}
	reg, err := registry.Load()
	if err != nil {
		return err
	}
	reg.UnbindMailVersion(version)
	return reg.Save()
}
