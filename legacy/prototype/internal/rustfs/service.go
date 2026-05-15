package rustfs

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
	displayName = "S3 Storage (RustFS)"
	serviceKey  = "s3"
	port        = 9000
	consolePort = 9001
)

type WebRoute struct {
	Subdomain string
	Port      int
}

func Binary() binaries.Binary { return binaries.Rustfs }
func Port() int               { return port }
func ConsolePort() int        { return consolePort }
func DisplayName() string     { return displayName }
func ServiceKey() string      { return serviceKey }

func WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "s3", Port: consolePort},
		{Subdomain: "s3-api", Port: port},
	}
}

func EnvVars(version, projectName string) (map[string]string, error) {
	if err := ValidateVersion(version); err != nil {
		return nil, err
	}
	return map[string]string{
		"AWS_ACCESS_KEY_ID":           "rstfsadmin",
		"AWS_SECRET_ACCESS_KEY":       "rstfsadmin",
		"AWS_DEFAULT_REGION":          "us-east-1",
		"AWS_BUCKET":                  projectName,
		"AWS_ENDPOINT":                "http://127.0.0.1:9000",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
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
	dataDir := config.RustfsDataDir(version)
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
	env := []string{
		"RUSTFS_ACCESS_KEY=rstfsadmin",
		"RUSTFS_SECRET_KEY=rstfsadmin",
	}
	return supervisor.Process{
		Name:         Binary().Name + "-" + version,
		Binary:       binPath,
		Args:         args,
		Env:          env,
		LogFile:      logFile,
		Ready:        ready,
		ReadyTimeout: rc.Timeout,
	}, nil
}
