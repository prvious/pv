package rustfs

import (
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

func BinaryPath(version string) (string, error) {
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return filepath.Join(config.RustfsBinDir(version), Binary().Name), nil
}

func LogPath(version string) (string, error) {
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return config.RustfsLogPath(version), nil
}

func IsInstalled(version string) bool {
	path, err := BinaryPath(version)
	if err != nil {
		return false
	}
	st, err := os.Stat(path)
	return err == nil && !st.IsDir()
}

func InstalledVersions() ([]string, error) {
	if IsInstalled(DefaultVersion()) {
		return []string{DefaultVersion()}, nil
	}
	return nil, nil
}
