package server

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

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

	// The full command line must include --console-enable. Without it, RustFS
	// does not bind port 9001 even though --console-address is set — this is
	// the exact regression we verified during Task 1. Assert it at the
	// supervisor-process layer, not just at the service layer.
	var sawConsoleEnable, sawConsoleAddress, sawAddress bool
	for i, a := range p.Args {
		switch a {
		case "--console-enable":
			sawConsoleEnable = true
		case "--console-address":
			if i+1 < len(p.Args) && p.Args[i+1] == ":9001" {
				sawConsoleAddress = true
			}
		case "--address":
			if i+1 < len(p.Args) && p.Args[i+1] == ":9000" {
				sawAddress = true
			}
		}
	}
	if !sawConsoleEnable {
		t.Errorf("Args missing --console-enable; got %v", p.Args)
	}
	if !sawConsoleAddress {
		t.Errorf("Args missing --console-address :9001; got %v", p.Args)
	}
	if !sawAddress {
		t.Errorf("Args missing --address :9000; got %v", p.Args)
	}
}

func TestBuildSupervisorProcess_Mailpit(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	svc := &services.Mailpit{}
	p, err := buildSupervisorProcess(svc)
	if err != nil {
		t.Fatalf("buildSupervisorProcess: %v", err)
	}
	// Binary name is "mailpit", not service name "mail".
	if p.Name != "mailpit" {
		t.Errorf("Name = %q, want mailpit (binary name, not service name)", p.Name)
	}
	if !strings.HasSuffix(p.Binary, "/internal/bin/mailpit") {
		t.Errorf("Binary = %q; should end with /internal/bin/mailpit", p.Binary)
	}
	if !strings.HasSuffix(p.LogFile, "/mailpit.log") {
		t.Errorf("LogFile = %q; expected .../mailpit.log", p.LogFile)
	}
	if p.ReadyTimeout != 30*time.Second {
		t.Errorf("ReadyTimeout = %v, want 30s", p.ReadyTimeout)
	}
	if p.Ready == nil {
		t.Error("Ready closure must be set")
	}
	// Verify Mailpit-specific args: --smtp :1025, --listen :8025, --database <dataDir>/mailpit.db
	var sawSMTP, sawListen, sawDatabase bool
	for i := 0; i < len(p.Args)-1; i++ {
		switch p.Args[i] {
		case "--smtp":
			if p.Args[i+1] == ":1025" {
				sawSMTP = true
			}
		case "--listen":
			if p.Args[i+1] == ":8025" {
				sawListen = true
			}
		case "--database":
			if strings.HasSuffix(p.Args[i+1], "/mailpit.db") {
				sawDatabase = true
			}
		}
	}
	if !sawSMTP {
		t.Errorf("Args missing --smtp :1025; got %v", p.Args)
	}
	if !sawListen {
		t.Errorf("Args missing --listen :8025; got %v", p.Args)
	}
	if !sawDatabase {
		t.Errorf("Args missing --database .../mailpit.db; got %v", p.Args)
	}
}

func TestBuildReadyFunc_RejectsZeroValue(t *testing.T) {
	// Zero-value ReadyCheck has neither tcpPort nor httpEndpoint set.
	// The "both set" case is now unconstructable from outside the services
	// package (fields are unexported) — the type system prevents it.
	_, err := buildReadyFunc(services.ReadyCheck{})
	if err == nil {
		t.Fatal("expected error for zero-value ReadyCheck")
	}
	if !strings.Contains(err.Error(), "exactly one") {
		t.Errorf("error should mention 'exactly one'; got %v", err)
	}
}

func TestBuildReadyFunc_TCPOnly(t *testing.T) {
	fn, err := buildReadyFunc(services.TCPReady(9000, 30*time.Second))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if fn == nil {
		t.Fatal("expected non-nil ready func")
	}
}

func TestBuildReadyFunc_HTTPOnly(t *testing.T) {
	fn, err := buildReadyFunc(services.HTTPReady("http://127.0.0.1:9000/health", 30*time.Second))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if fn == nil {
		t.Fatal("expected non-nil ready func")
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
