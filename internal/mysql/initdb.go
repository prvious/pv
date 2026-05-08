package mysql

import (
	"fmt"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// RunInitdb invokes the bundled mysqld with --initialize-insecure to
// populate the per-version data dir. Idempotent: if auto.cnf already
// exists, returns nil immediately (auto.cnf is the durable marker that
// --initialize-insecure ran successfully). On failure, removes the
// partially-created data dir so retry is clean.
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
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return fmt.Errorf("create data dir: %w", err)
	}

	binPath := filepath.Join(config.MysqlBinDir(version), "mysqld")
	basedir := config.MysqlVersionDir(version)
	args := []string{
		"--initialize-insecure",
		"--datadir=" + dataDir,
		"--basedir=" + basedir,
	}
	// mysqld refuses to run as root unless --user is passed. Use the
	// current user's name; this is a no-op when not root, and matches
	// the spec's `--user=<current-user>` requirement when sudo'd.
	if u, err := user.Current(); err == nil && u.Username != "" {
		args = append(args, "--user="+u.Username)
	}

	cmd := exec.Command(binPath, args...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		os.RemoveAll(dataDir)
		return fmt.Errorf("mysqld --initialize-insecure failed: %w\n%s", err, out)
	}
	return nil
}
