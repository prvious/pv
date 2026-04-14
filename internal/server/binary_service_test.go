package server

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/supervisor"
)

func TestBuildSupervisorProcess_RustFS(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	svc := &services.RustFS{}
	p, err := buildSupervisorProcess(svc)
	if err != nil {
		t.Fatalf("buildSupervisorProcess: %v", err)
	}
	if p.Name != "rustfs" {
		t.Errorf("Name = %q, want rustfs", p.Name)
	}
	if !strings.HasSuffix(p.Binary, "/internal/bin/rustfs") {
		t.Errorf("Binary = %q; should end with /internal/bin/rustfs", p.Binary)
	}
	if !strings.Contains(p.LogFile, "logs") || !strings.HasSuffix(p.LogFile, "/rustfs.log") {
		t.Errorf("LogFile = %q; expected ~/.pv/logs/rustfs.log", p.LogFile)
	}
	// Data dir should be created on the fly.
	dataDir := ""
	for i, a := range p.Args {
		if a == "server" && i+1 < len(p.Args) {
			dataDir = p.Args[i+1]
			break
		}
	}
	if dataDir == "" {
		t.Fatal("could not find data dir in Args")
	}
	if _, err := os.Stat(dataDir); err != nil {
		t.Errorf("data dir %s should exist: %v", dataDir, err)
	}
	if p.Ready == nil {
		t.Error("Ready closure must be set")
	}
	if p.ReadyTimeout == 0 {
		t.Error("ReadyTimeout must be non-zero")
	}
}

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
