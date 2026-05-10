package server

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/supervisor"
)

func TestWriteDaemonStatus_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := supervisor.New()
	// No live processes — we just test the file write path.
	if err := writeDaemonStatus(s); err != nil {
		t.Fatalf("writeDaemonStatus: %v", err)
	}
	path := filepath.Join(os.Getenv("HOME"), ".pv", "daemon-status.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read daemon-status.json: %v", err)
	}
	var snap DaemonStatus
	if err := json.Unmarshal(data, &snap); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if snap.PID != os.Getpid() {
		t.Errorf("PID = %d, want %d", snap.PID, os.Getpid())
	}
	if snap.Supervised == nil {
		t.Error("Supervised map should be initialized")
	}
}
