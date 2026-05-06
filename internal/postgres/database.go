package postgres

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strconv"

	"github.com/prvious/pv/internal/config"
)

// CreateDatabase creates dbName on the given postgres major using the
// bundled psql via absolute path. Idempotent: a SELECT-then-CREATE pattern
// avoids "database already exists" errors.
func CreateDatabase(major, dbName string) error {
	port, err := PortFor(major)
	if err != nil {
		return err
	}
	psql := filepath.Join(config.PostgresBinDir(major), "psql")
	args := []string{
		"-h", "127.0.0.1",
		"-p", strconv.Itoa(port),
		"-U", "postgres",
		"-tAc",
		fmt.Sprintf("SELECT 1 FROM pg_database WHERE datname = '%s'", dbName),
	}
	out, err := exec.Command(psql, args...).Output()
	if err != nil {
		return fmt.Errorf("psql probe: %w", err)
	}
	if string(out) == "1\n" {
		return nil
	}
	createArgs := []string{
		"-h", "127.0.0.1",
		"-p", strconv.Itoa(port),
		"-U", "postgres",
		"-c",
		fmt.Sprintf(`CREATE DATABASE "%s"`, dbName),
	}
	if _, err := exec.Command(psql, createArgs...).Output(); err != nil {
		return fmt.Errorf("psql create: %w", err)
	}
	return nil
}
