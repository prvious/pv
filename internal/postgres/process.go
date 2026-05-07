package postgres

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/supervisor"
)

// BuildSupervisorProcess returns a supervisor.Process for a postgres major.
// Refuses to build for a data dir without PG_VERSION (i.e., not yet
// initialized). Port comes from postgresql.conf — not the command line —
// so there's a single source of truth.
func BuildSupervisorProcess(major string) (supervisor.Process, error) {
	dataDir := config.ServiceDataDir("postgres", major)
	if _, err := os.Stat(filepath.Join(dataDir, "PG_VERSION")); err != nil {
		return supervisor.Process{}, fmt.Errorf("postgres %s: data dir not initialized (run pv postgres:install %s)", major, major)
	}
	port, err := PortFor(major)
	if err != nil {
		return supervisor.Process{}, err
	}
	// /tmp can be reaped between boots, so the socket dir referenced by
	// postgresql.conf may not exist when the supervisor respawns. Recreate
	// it on every build — postgres won't auto-create it and FATALs out
	// otherwise ("could not create lock file: No such file or directory").
	if err := os.MkdirAll(socketDir(major), 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create socket dir: %w", err)
	}
	binary := filepath.Join(config.PostgresBinDir(major), "postgres")
	return supervisor.Process{
		Name:         "postgres-" + major,
		Binary:       binary,
		Args:         []string{"-D", dataDir},
		LogFile:      config.PostgresLogPath(major),
		SysProcAttr:  dropSysProcAttr(),
		Ready:        tcpReady(port),
		ReadyTimeout: 30 * time.Second,
	}, nil
}

// socketDir returns the /tmp directory used as unix_socket_directories
// for a postgres major. Must match what conf.go writes to postgresql.conf.
func socketDir(major string) string {
	return "/tmp/pv-postgres-" + major
}

func tcpReady(port int) func(context.Context) error {
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	return func(ctx context.Context) error {
		d := net.Dialer{Timeout: 500 * time.Millisecond}
		c, err := d.DialContext(ctx, "tcp", addr)
		if err != nil {
			return err
		}
		c.Close()
		return nil
	}
}
