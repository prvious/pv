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
	binary := filepath.Join(config.PostgresBinDir(major), "postgres")
	return supervisor.Process{
		Name:         "postgres-" + major,
		Binary:       binary,
		Args:         []string{"-D", dataDir},
		LogFile:      config.PostgresLogPath(major),
		Ready:        tcpReady(port),
		ReadyTimeout: 30 * time.Second,
	}, nil
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
