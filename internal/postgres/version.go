package postgres

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/config"
)

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
