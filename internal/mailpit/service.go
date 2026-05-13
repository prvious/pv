package mailpit

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
	displayName = "Mail (Mailpit)"
	serviceKey  = "mail"
	port        = 1025
	consolePort = 8025
)

type WebRoute struct {
	Subdomain string
	Port      int
}

func Binary() binaries.Binary { return binaries.Mailpit }
func Port() int               { return port }
func ConsolePort() int        { return consolePort }
func DisplayName() string     { return displayName }
func ServiceKey() string      { return serviceKey }

func WebRoutes() []WebRoute {
	return []WebRoute{{Subdomain: "mail", Port: consolePort}}
}

func EnvVars(version, _ string) (map[string]string, error) {
	if err := ValidateVersion(version); err != nil {
		return nil, err
	}
	return map[string]string{
		"MAIL_MAILER":   "smtp",
		"MAIL_HOST":     "127.0.0.1",
		"MAIL_PORT":     "1025",
		"MAIL_USERNAME": "",
		"MAIL_PASSWORD": "",
	}, nil
}

func BuildSupervisorProcess(version string) (supervisor.Process, error) {
	if err := ValidateVersion(version); err != nil {
		return supervisor.Process{}, err
	}
	binPath, err := BinaryPath(version)
	if err != nil {
		return supervisor.Process{}, err
	}
	dataDir := config.ServiceDataDir(serviceKey, version)
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create data dir %s: %w", dataDir, err)
	}
	logFile, err := LogPath(version)
	if err != nil {
		return supervisor.Process{}, err
	}
	if err := os.MkdirAll(filepath.Dir(logFile), 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create log dir: %w", err)
	}
	rc := supervisor.HTTPReady(fmt.Sprintf("http://127.0.0.1:%d/livez", consolePort), 30*time.Second)
	ready, err := supervisor.BuildReadyFunc(rc)
	if err != nil {
		return supervisor.Process{}, fmt.Errorf("mailpit: %w", err)
	}
	args := []string{
		"--smtp", fmt.Sprintf(":%d", port),
		"--listen", fmt.Sprintf(":%d", consolePort),
		"--database", dataDir + "/mailpit.db",
	}
	return supervisor.Process{
		Name:         Binary().Name + "-" + version,
		Binary:       binPath,
		Args:         args,
		Env:          nil,
		LogFile:      logFile,
		Ready:        ready,
		ReadyTimeout: rc.Timeout,
	}, nil
}
