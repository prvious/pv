package postgres

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// RunInitdb runs the bundled initdb against the per-major data dir.
// Idempotent: if PG_VERSION is already present, returns nil immediately.
// On failure, removes the partially-created data dir so retry is clean.
//
// When pv runs as root (e.g. via `sudo pv start` to bind :443), initdb
// still refuses to run as root. We drop to SUDO_UID before exec — and
// chown the data dir's parent first so the dropped user can write into it.
func RunInitdb(major string) error {
	dataDir := config.ServiceDataDir("postgres", major)
	pgVersion := filepath.Join(dataDir, "PG_VERSION")
	if _, err := os.Stat(pgVersion); err == nil {
		return nil
	}
	parent := filepath.Dir(dataDir)
	if err := os.MkdirAll(parent, 0o755); err != nil {
		return fmt.Errorf("create services dir: %w", err)
	}
	if err := chownToTarget(parent); err != nil {
		return fmt.Errorf("chown services dir: %w", err)
	}

	binPath := filepath.Join(config.PostgresBinDir(major), "initdb")
	cmd := exec.Command(binPath,
		"-D", dataDir,
		"-U", "postgres",
		"--auth=trust",
		"--encoding=UTF8",
		"--locale=C",
	)
	cmd.SysProcAttr = dropSysProcAttr()
	out, err := cmd.CombinedOutput()
	if err != nil {
		os.RemoveAll(dataDir)
		return fmt.Errorf("initdb failed: %w\n%s", err, out)
	}
	return nil
}
