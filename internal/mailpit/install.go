package mailpit

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
	if err := os.MkdirAll(config.MailpitDataDir(version), 0o755); err != nil {
		return fmt.Errorf("create mailpit data dir: %w", err)
	}
	if err := recordInstalledVersion(version); err != nil {
		return err
	}
	return SetWanted(version, WantedRunning)
}

func installArchive(client *http.Client, version string, progress binaries.ProgressFunc) error {
	url, err := binaries.MailpitURL(version)
	if err != nil {
		return err
	}
	return binaries.InstallVersionedArchive(client, binaries.VersionedArchiveInstall{
		ArtifactName: "mailpit",
		URL:          url,
		ArchivePath:  filepath.Join(config.PvDir(), "mailpit-"+version+".tar.gz"),
		VersionDir:   config.MailpitVersionDir(version),
		BinaryName:   Binary().Name,
		Progress:     progress,
	})
}

func recordInstalledVersion(version string) error {
	recorded, err := binaries.ReadArtifactVersion(config.MailpitVersionDir(version), "mailpit")
	if err != nil {
		return err
	}
	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set("mailpit-"+version, recorded)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}
	return nil
}
