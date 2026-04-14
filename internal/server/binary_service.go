package server

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/supervisor"
)

// DaemonStatus is the JSON snapshot written to ~/.pv/daemon-status.json.
type DaemonStatus struct {
	PID        int                         `json:"pid"`
	StartedAt  time.Time                   `json:"started_at"`
	Supervised map[string]SupervisedStatus `json:"supervised"`
}

// SupervisedStatus holds the runtime state of a single supervised binary.
type SupervisedStatus struct {
	PID     int  `json:"pid"`
	Running bool `json:"running"`
}

// daemonStartedAt is captured when the package is first initialized inside the
// daemon process. It is recorded in every status snapshot.
var daemonStartedAt = time.Now()

// buildSupervisorProcess translates a BinaryService into a supervisor.Process.
// It resolves all paths via internal/config and creates the data + log directories.
func buildSupervisorProcess(svc services.BinaryService) (supervisor.Process, error) {
	binaryName := svc.Binary().Name
	binaryPath := filepath.Join(config.InternalBinDir(), binaryName)

	dataDir := config.ServiceDataDir(svc.Name(), "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create data dir %s: %w", dataDir, err)
	}

	logFile := filepath.Join(config.PvDir(), "logs", binaryName+".log")
	if err := os.MkdirAll(filepath.Dir(logFile), 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create log dir: %w", err)
	}

	rc := svc.ReadyCheck()
	ready, err := buildReadyFunc(rc)
	if err != nil {
		return supervisor.Process{}, fmt.Errorf("%s: %w", svc.Name(), err)
	}

	return supervisor.Process{
		Name:         binaryName,
		Binary:       binaryPath,
		Args:         svc.Args(dataDir),
		Env:          svc.Env(),
		LogFile:      logFile,
		Ready:        ready,
		ReadyTimeout: rc.Timeout,
	}, nil
}

// buildReadyFunc returns a ReadyFunc appropriate to the ReadyCheck variant.
// The ReadyCheck must specify exactly one of TCPPort or HTTPEndpoint — the
// doc comment on the type promises this; we enforce it here rather than
// silently treating a zero-value as "instantly ready" (which would let a
// misconfigured BinaryService bypass the probe entirely).
func buildReadyFunc(rc services.ReadyCheck) (func(context.Context) error, error) {
	httpSet := rc.HTTPEndpoint != ""
	tcpSet := rc.TCPPort > 0
	switch {
	case httpSet && tcpSet:
		return nil, fmt.Errorf("invalid ReadyCheck: both TCPPort and HTTPEndpoint set; specify exactly one")
	case httpSet:
		client := &http.Client{Timeout: 2 * time.Second}
		url := rc.HTTPEndpoint
		return func(ctx context.Context) error {
			req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
			if err != nil {
				return err
			}
			resp, err := client.Do(req)
			if err != nil {
				return err
			}
			defer resp.Body.Close()
			if resp.StatusCode >= 200 && resp.StatusCode < 300 {
				return nil
			}
			return fmt.Errorf("HTTP %s returned %d", url, resp.StatusCode)
		}, nil
	case tcpSet:
		addr := fmt.Sprintf("127.0.0.1:%d", rc.TCPPort)
		return func(ctx context.Context) error {
			d := net.Dialer{Timeout: 500 * time.Millisecond}
			c, err := d.DialContext(ctx, "tcp", addr)
			if err != nil {
				return err
			}
			c.Close()
			return nil
		}, nil
	default:
		return nil, fmt.Errorf("invalid ReadyCheck: must set exactly one of TCPPort or HTTPEndpoint")
	}
}

// writeDaemonStatus serializes the current supervisor state to
// ~/.pv/daemon-status.json. Safe to call from the reconcile path.
func writeDaemonStatus(sup *supervisor.Supervisor) error {
	snap := DaemonStatus{
		PID:        os.Getpid(),
		StartedAt:  daemonStartedAt,
		Supervised: map[string]SupervisedStatus{},
	}
	if sup != nil {
		for _, name := range sup.SupervisedNames() {
			snap.Supervised[name] = SupervisedStatus{
				PID:     sup.Pid(name),
				Running: sup.IsRunning(name),
			}
		}
	}
	data, err := json.MarshalIndent(snap, "", "  ")
	if err != nil {
		return err
	}
	path := filepath.Join(config.PvDir(), "daemon-status.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, data, 0o644)
}

// ReadDaemonStatus returns the parsed ~/.pv/daemon-status.json, or nil+error if
// the file is missing or corrupt or if the recorded PID is not alive.
func ReadDaemonStatus() (*DaemonStatus, error) {
	path := filepath.Join(config.PvDir(), "daemon-status.json")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var snap DaemonStatus
	if err := json.Unmarshal(data, &snap); err != nil {
		return nil, err
	}
	// Liveness check.
	if proc, err := os.FindProcess(snap.PID); err == nil {
		if err := proc.Signal(syscall.Signal(0)); err != nil {
			return nil, fmt.Errorf("daemon-status.json is stale (pid %d not alive)", snap.PID)
		}
	}
	return &snap, nil
}
