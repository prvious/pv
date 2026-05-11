package mysql

import (
	"fmt"
	"os/exec"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// CreateDatabase creates dbName on the given mysql version using the bundled
// `mysql` client over the unix socket. Idempotent via
// `CREATE DATABASE IF NOT EXISTS`. The dbName is backquoted so identifiers
// with dots or hyphens (typical when projectName comes from a slugified
// directory) parse correctly.
func CreateDatabase(version, dbName string) error {
	bin := filepath.Join(config.MysqlBinDir(version), "mysql")
	socket := fmt.Sprintf("/tmp/pv-mysql-%s.sock", version)
	stmt := fmt.Sprintf("CREATE DATABASE IF NOT EXISTS `%s`;", dbName)
	args := []string{
		"--socket=" + socket,
		"-u", "root",
		"-e", stmt,
	}
	out, err := exec.Command(bin, args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("mysql create database %q: %w (output: %s)", dbName, err, string(out))
	}
	return nil
}

// DropDatabase drops dbName on the given mysql version using the bundled
// `mysql` client over the unix socket. Idempotent via
// `DROP DATABASE IF EXISTS`. The dbName is backquoted so identifiers with
// dots or hyphens (typical when projectName comes from a slugified
// directory) parse correctly.
func DropDatabase(version, dbName string) error {
	bin := filepath.Join(config.MysqlBinDir(version), "mysql")
	socket := fmt.Sprintf("/tmp/pv-mysql-%s.sock", version)
	stmt := fmt.Sprintf("DROP DATABASE IF EXISTS `%s`;", dbName)
	args := []string{
		"--socket=" + socket,
		"-u", "root",
		"-e", stmt,
	}
	out, err := exec.Command(bin, args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("mysql drop database %q: %w (output: %s)", dbName, err, string(out))
	}
	return nil
}
