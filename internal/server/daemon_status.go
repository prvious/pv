package server

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/config"
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
