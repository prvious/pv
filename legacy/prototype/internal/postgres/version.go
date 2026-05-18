package postgres

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/config"
)

const defaultVersion = "18"

func DefaultVersion() string { return defaultVersion }

func ResolveVersion(version string) (string, error) {
	if version == "" {
		return DefaultVersion(), nil
	}
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return version, nil
}

func ValidateVersion(version string) error {
	switch version {
	case "17", "18":
		return nil
	default:
		return fmt.Errorf("unsupported postgres version %q (want one of 17, 18)", version)
	}
}

// ProbeVersion runs `<bin>/pg_config --version` and returns the version
// component (e.g. "17.5" from "PostgreSQL 17.5"). The major argument
// selects the install root; the answer may be a patch within that major.
func ProbeVersion(major string) (string, error) {
	binPath := filepath.Join(config.PostgresBinDir(major), "pg_config")
	out, err := exec.Command(binPath, "--version").Output()
	if err != nil {
		return "", fmt.Errorf("pg_config --version: %w", err)
	}
	s := strings.TrimSpace(string(out))
	const prefix = "PostgreSQL "
	if !strings.HasPrefix(s, prefix) {
		return "", fmt.Errorf("unexpected pg_config output: %q", s)
	}
	return strings.TrimPrefix(s, prefix), nil
}
