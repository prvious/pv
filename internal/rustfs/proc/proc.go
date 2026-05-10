// Package proc provides the supervisor.Process builder for rustfs.
// It is a leaf package (no dependency on internal/server) so that
// internal/server can import it without creating an import cycle with the
// parent internal/rustfs package (which does import internal/server).
package proc

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/supervisor"
)

const (
	serviceKey  = "s3"
	port        = 9000
	consolePort = 9001
)

// ServiceKey returns the registry key for rustfs.
func ServiceKey() string { return serviceKey }

// Port returns the API port rustfs listens on.
func Port() int { return port }

// ConsolePort returns the web-console port rustfs listens on.
func ConsolePort() int { return consolePort }

// WebRoute maps a subdomain under pv.{tld} to a local port.
// It mirrors caddy.WebRoute but is defined here to keep the proc package free
// of a caddy import (which would create an import cycle when caddy imports proc).
type WebRoute struct {
	Subdomain string
	Port      int
}

// WebRoutes returns the reverse-proxy routes that rustfs exposes.
func WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "s3", Port: consolePort},
		{Subdomain: "s3-api", Port: port},
	}
}

// Binary returns the binaries.Binary descriptor for rustfs.
func Binary() binaries.Binary { return binaries.Rustfs }

// BuildSupervisorProcess returns the supervisor.Process for rustfs.
func BuildSupervisorProcess() (supervisor.Process, error) {
	binPath := filepath.Join(config.InternalBinDir(), Binary().Name)

	dataDir := config.ServiceDataDir(serviceKey, "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create data dir %s: %w", dataDir, err)
	}

	logFile := filepath.Join(config.PvDir(), "logs", Binary().Name+".log")
	if err := os.MkdirAll(filepath.Dir(logFile), 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create log dir: %w", err)
	}

	rc := supervisor.TCPReady(port, 30*time.Second)
	ready, err := supervisor.BuildReadyFunc(rc)
	if err != nil {
		return supervisor.Process{}, fmt.Errorf("rustfs: %w", err)
	}

	args := []string{
		"server", dataDir,
		"--address", fmt.Sprintf(":%d", port),
		"--console-enable",
		"--console-address", fmt.Sprintf(":%d", consolePort),
	}
	// RUSTFS_ACCESS_KEY / RUSTFS_SECRET_KEY are the env var names rustfs expects
	// per `rustfs server --help`. ROOT_USER / ROOT_PASSWORD (the MinIO equivalents)
	// are NOT recognised by RustFS — don't substitute them.
	env := []string{
		"RUSTFS_ACCESS_KEY=rstfsadmin",
		"RUSTFS_SECRET_KEY=rstfsadmin",
	}

	return supervisor.Process{
		Name:         Binary().Name,
		Binary:       binPath,
		Args:         args,
		Env:          env,
		LogFile:      logFile,
		Ready:        ready,
		ReadyTimeout: rc.Timeout,
	}, nil
}
