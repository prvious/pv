// Package proc provides the supervisor.Process builder for mailpit.
// It is a leaf package (no dependency on internal/server) so that
// internal/server can import it without creating an import cycle with the
// parent internal/mailpit package (which does import internal/server).
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
	// serviceKey must remain "mail" (not "mailpit") to preserve compatibility
	// with the previous Docker-based mail service — renaming breaks existing
	// pv.yml references and linked projects' .env files.
	serviceKey  = "mail"
	port        = 1025
	consolePort = 8025
)

// WebRoute maps a subdomain under pv.{tld} to a local port.
// It mirrors caddy.WebRoute but is defined here to keep the proc package free
// of a caddy import (which would create an import cycle when caddy imports proc).
type WebRoute struct {
	Subdomain string
	Port      int
}

// WebRoutes returns the reverse-proxy routes that mailpit exposes.
func WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "mail", Port: consolePort},
	}
}

// Binary returns the binaries.Binary descriptor for mailpit.
func Binary() binaries.Binary { return binaries.Mailpit }

// BuildSupervisorProcess returns the supervisor.Process for mailpit.
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

	// ReadyCheck uses Mailpit's documented /livez endpoint, which returns 200 once
	// both the SMTP and HTTP servers are listening.
	rc := supervisor.HTTPReady("http://127.0.0.1:8025/livez", 30*time.Second)
	ready, err := supervisor.BuildReadyFunc(rc)
	if err != nil {
		return supervisor.Process{}, fmt.Errorf("mailpit: %w", err)
	}

	// Args pins the SMTP and HTTP bind addresses to :1025 and :8025; the values
	// must agree with Port() and ConsolePort() or MAIL_PORT / WebRoutes drift.
	// Flag names match `mailpit --help` for v1.29.6.
	args := []string{
		"--smtp", fmt.Sprintf(":%d", port),
		"--listen", fmt.Sprintf(":%d", consolePort),
		"--database", dataDir + "/mailpit.db",
	}

	return supervisor.Process{
		Name:         Binary().Name,
		Binary:       binPath,
		Args:         args,
		Env:          nil,
		LogFile:      logFile,
		Ready:        ready,
		ReadyTimeout: rc.Timeout,
	}, nil
}
