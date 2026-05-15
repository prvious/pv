package rustfs

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
	if err := SetWanted(version, WantedStopped); err != nil {
		return fmt.Errorf("stop rustfs %s: %w", version, err)
	}
	if err := WaitStopped(version, 30*time.Second); err != nil {
		return fmt.Errorf("wait for rustfs %s to stop: %w", version, err)
	}

	if err := os.RemoveAll(config.RustfsVersionDir(version)); err != nil {
		return fmt.Errorf("remove rustfs version dir: %w", err)
	}
	if logPath, err := LogPath(version); err == nil {
		_ = os.Remove(logPath)
	}
	if err := RemoveVersion(version); err != nil {
		return err
	}
	if vs, err := binaries.LoadVersions(); err == nil {
		delete(vs.Versions, "rustfs-"+version)
		if err := vs.Save(); err != nil {
			return fmt.Errorf("save versions state: %w", err)
		}
	}
	if force {
		if err := os.RemoveAll(config.RustfsDataDir(version)); err != nil {
			return fmt.Errorf("cannot delete data: %w", err)
		}
	}
	reg, err := registry.Load()
	if err != nil {
		return err
	}
	reg.UnbindS3Version(version)
	return reg.Save()
}
