package mysql

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// RunInitdb invokes the bundled mysqld with --initialize-insecure to
// populate the per-version data dir. Idempotent: if auto.cnf already
// exists, returns nil immediately. On failure, removes the
// partially-created data dir so retry is clean.
//
// When pv runs as root (e.g. via `sudo pv start` to bind :443), we drop
// to SUDO_UID before exec — and chown the parent + data dir first so
// the dropped user can write into them. This keeps file ownership
// consistent with the supervisor's later mysqld process (which also
// runs with dropped privileges).
func RunInitdb(version string) error {
	dataDir := config.MysqlDataDir(version)
	autoCnf := filepath.Join(dataDir, "auto.cnf")
	if _, err := os.Stat(autoCnf); err == nil {
		return nil
	}

	parent := filepath.Dir(dataDir)
	if err := os.MkdirAll(parent, 0o755); err != nil {
		return fmt.Errorf("create mysql data parent dir: %w", err)
	}
	if err := chownToTarget(parent); err != nil {
		return fmt.Errorf("chown mysql data parent: %w", err)
	}
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return fmt.Errorf("create data dir: %w", err)
	}
	if err := chownToTarget(dataDir); err != nil {
		return fmt.Errorf("chown mysql data dir: %w", err)
	}

	binPath := filepath.Join(config.MysqlBinDir(version), "mysqld")
	basedir := config.MysqlVersionDir(version)
	cmd := exec.Command(binPath,
		"--initialize-insecure",
		"--datadir="+dataDir,
		"--basedir="+basedir,
	)
	cmd.SysProcAttr = dropSysProcAttr()
	out, err := cmd.CombinedOutput()
	if err != nil {
		os.RemoveAll(dataDir)
		return fmt.Errorf("mysqld --initialize-insecure failed: %w\n%s", err, out)
	}
	return nil
}
