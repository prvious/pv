package rustfs

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
	if err := ValidateVersion(version); err != nil {
		return err
	}
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	if err := installArchive(client, version, progress); err != nil {
		return err
	}
	if err := os.MkdirAll(config.RustfsDataDir(version), 0o755); err != nil {
		return fmt.Errorf("create rustfs data dir: %w", err)
	}
	if err := recordInstalledVersion(version); err != nil {
		return err
	}
	return SetWanted(version, WantedRunning)
}

func installArchive(client *http.Client, version string, progress binaries.ProgressFunc) error {
	url, err := binaries.RustfsURL(version)
	if err != nil {
		return err
	}
	return binaries.InstallVersionedArchive(client, binaries.VersionedArchiveInstall{
		ArtifactName: "rustfs",
		URL:          url,
		ArchivePath:  filepath.Join(config.PvDir(), "rustfs-"+version+".tar.gz"),
		VersionDir:   config.RustfsVersionDir(version),
		BinaryName:   Binary().Name,
		Progress:     progress,
	})
}

func recordInstalledVersion(version string) error {
	recorded, err := binaries.ReadArtifactVersion(config.RustfsVersionDir(version), "rustfs")
	if err != nil {
		return err
	}
	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set("rustfs-"+version, recorded)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}
	return nil
}
