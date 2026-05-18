package postgres

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"

	"github.com/prvious/pv/internal/config"
)

const (
	overridesBeginMarker = "# pv-managed begin"
	overridesEndMarker   = "# pv-managed end"
)

// pvManagedBlock matches our managed block (begin to end, inclusive of any
// content/newlines between). Used to strip the previous block before
// re-appending so multiple WriteOverrides calls don't pile up.
var pvManagedBlock = regexp.MustCompile(`(?ms)\n?# pv-managed begin.*?# pv-managed end\n`)

// WriteOverrides appends pv's postgresql.conf overrides for a major,
// replacing any previously-written pv block. Safe to call repeatedly.
func WriteOverrides(major string) error {
	port, err := PortFor(major)
	if err != nil {
		return err
	}
	confPath := filepath.Join(config.ServiceDataDir("postgres", major), "postgresql.conf")
	current, err := os.ReadFile(confPath)
	if err != nil {
		return fmt.Errorf("read postgresql.conf: %w", err)
	}
	stripped := pvManagedBlock.ReplaceAll(current, []byte("\n"))
	block := fmt.Sprintf(`
%s
# Managed by pv — do not hand-edit.
listen_addresses = '127.0.0.1'
port = %d
unix_socket_directories = '/tmp/pv-postgres-%s'
fsync = on
synchronous_commit = on
logging_collector = off
log_destination = 'stderr'
shared_buffers = 128MB
max_connections = 100
%s
`, overridesBeginMarker, port, major, overridesEndMarker)
	out := append(stripped, []byte(block)...)
	return os.WriteFile(confPath, out, 0o644)
}

// RewriteHBA writes the trust-only pg_hba.conf for a major.
// Loopback only — no external network exposure.
func RewriteHBA(major string) error {
	hbaPath := filepath.Join(config.ServiceDataDir("postgres", major), "pg_hba.conf")
	body := []byte(`# Managed by pv — do not hand-edit.
# TYPE  DATABASE        USER            ADDRESS                 METHOD
local   all             all                                     trust
host    all             all             127.0.0.1/32            trust
host    all             all             ::1/128                 trust
`)
	return os.WriteFile(hbaPath, body, 0o600)
}
