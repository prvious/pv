package postgres

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// CreateDatabase creates dbName on the given postgres major using the
// bundled psql via absolute path. Idempotent: inspects psql's stderr for
// the "already exists" message and treats it as success, avoiding the
// TOCTOU race of a SELECT-then-CREATE pattern. CREATE DATABASE cannot
// run inside DO $$ ... EXCEPTION $$ blocks, so the canonical idempotent
// idiom is to attempt the CREATE and swallow the well-known error.
func CreateDatabase(major, dbName string) error {
	port, err := PortFor(major)
	if err != nil {
		return err
	}
	psql := filepath.Join(config.PostgresBinDir(major), "psql")
	createArgs := []string{
		"-h", "127.0.0.1",
		"-p", strconv.Itoa(port),
		"-U", "postgres",
		"-c",
		fmt.Sprintf(`CREATE DATABASE "%s"`, dbName),
	}
	out, err := exec.Command(psql, createArgs...).CombinedOutput()
	if err != nil {
		if strings.Contains(string(out), "already exists") {
			return nil
		}
		return fmt.Errorf("create postgres database %q: %w (output: %s)", dbName, err, string(out))
	}
	return nil
}

// DropDatabase drops dbName on the given postgres major using the bundled
// psql via absolute path. Idempotent: uses `DROP DATABASE IF EXISTS` so it
// is a no-op when the database does not exist.
func DropDatabase(major, dbName string) error {
	port, err := PortFor(major)
	if err != nil {
		return err
	}
	psql := filepath.Join(config.PostgresBinDir(major), "psql")
	args := []string{
		"-h", "127.0.0.1",
		"-p", strconv.Itoa(port),
		"-U", "postgres",
		"-c",
		fmt.Sprintf(`DROP DATABASE IF EXISTS "%s"`, dbName),
	}
	out, err := exec.Command(psql, args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("drop postgres database %q: %w (output: %s)", dbName, err, string(out))
	}
	return nil
}
