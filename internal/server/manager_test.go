package server

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/supervisor"
)

func TestReconcile_SpawnsBinaryServices(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Seed a registry with s3 as a binary service.
	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "binary", Port: 9000, ConsolePort: 9001, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	// Put a fake "rustfs" binary in place so supervisor spawn doesn't ENOENT.
	binDir := config.InternalBinDir()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	fakeBin := filepath.Join(binDir, "rustfs")
	// The fake binary must bind port 9000 so the supervisor ready-check succeeds.
	script := "#!/usr/bin/env python3\nimport socket, time\ns=socket.socket()\ns.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\ns.bind(('127.0.0.1', 9000))\ns.listen(1)\ntime.sleep(60)\n"
	if err := os.WriteFile(fakeBin, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	m := &ServerManager{supervisor: sup, secondaries: map[string]*FrankenPHP{}}
	defer m.supervisor.StopAll(2 * time.Second)

	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}
	if !sup.IsRunning("rustfs") {
		t.Error("expected rustfs to be supervised after reconcile")
	}
}

func TestReconcile_StopsDisabledBinaryServices(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	binDir := config.InternalBinDir()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	// The fake binary must bind port 9000 so the supervisor ready-check succeeds.
	script := "#!/usr/bin/env python3\nimport socket, time\ns=socket.socket()\ns.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)\ns.bind(('127.0.0.1', 9000))\ns.listen(1)\ntime.sleep(60)\n"
	if err := os.WriteFile(filepath.Join(binDir, "rustfs"), []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}

	sup := supervisor.New()
	m := &ServerManager{supervisor: sup, secondaries: map[string]*FrankenPHP{}}
	defer sup.StopAll(2 * time.Second)

	// Phase 1: enabled, should start.
	enabled := true
	reg1 := &registry.Registry{Services: map[string]*registry.ServiceInstance{
		"s3": {Kind: "binary", Port: 9000, Enabled: &enabled},
	}}
	if err := reg1.Save(); err != nil {
		t.Fatal(err)
	}
	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !sup.IsRunning("rustfs") {
		t.Fatal("expected rustfs running after first reconcile")
	}

	// Phase 2: disabled, should stop.
	disabled := false
	reg2 := &registry.Registry{Services: map[string]*registry.ServiceInstance{
		"s3": {Kind: "binary", Port: 9000, Enabled: &disabled},
	}}
	if err := reg2.Save(); err != nil {
		t.Fatal(err)
	}
	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if sup.IsRunning("rustfs") {
		t.Error("expected rustfs stopped after disabling via reconcile")
	}
}
