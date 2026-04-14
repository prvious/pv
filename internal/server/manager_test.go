package server

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/supervisor"
)

// fakeBinaryBuildOnce caches the compiled fake-binary path across tests in
// this package — go build is expensive and the helper is stateless.
var (
	fakeBinaryPathOnce sync.Once
	fakeBinaryPath     string
	fakeBinaryErr      error
)

// compiledFakeBinary compiles testdata/fakebinary/main.go once per test run
// and returns the absolute path to the resulting executable. The binary
// binds a TCP port on 127.0.0.1 and sleeps — matching what the RustFS
// supervisor expects for its TCP ready-check.
func compiledFakeBinary(t *testing.T) string {
	t.Helper()
	fakeBinaryPathOnce.Do(func() {
		dir, err := os.MkdirTemp("", "pv-fake-binary-*")
		if err != nil {
			fakeBinaryErr = err
			return
		}
		// Keep this around for the life of the test process; the OS will
		// clean it up or leave it in the user's tmp — either is fine.
		out := filepath.Join(dir, "fakebinary")
		src := filepath.Join("testdata", "fakebinary", "main.go")
		cmd := exec.Command("go", "build", "-o", out, src)
		if output, err := cmd.CombinedOutput(); err != nil {
			fakeBinaryErr = err
			t.Logf("go build output: %s", output)
			return
		}
		fakeBinaryPath = out
	})
	if fakeBinaryErr != nil {
		t.Fatalf("compile fake binary: %v", fakeBinaryErr)
	}
	return fakeBinaryPath
}

// stageFakeBinaryAsRustfs copies the compiled fake binary into
// ~/.pv/internal/bin/rustfs so the supervisor finds it via the normal path.
func stageFakeBinaryAsRustfs(t *testing.T) {
	t.Helper()
	src := compiledFakeBinary(t)
	binDir := config.InternalBinDir()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(src)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(binDir, "rustfs"), data, 0o755); err != nil {
		t.Fatal(err)
	}
}

func TestReconcile_SpawnsBinaryServices(t *testing.T) {
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
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

	stageFakeBinaryAsRustfs(t)

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
	if runtime.GOOS != "darwin" && runtime.GOOS != "linux" {
		t.Skipf("fake binary helper not supported on %s", runtime.GOOS)
	}
	t.Setenv("HOME", t.TempDir())
	stageFakeBinaryAsRustfs(t)

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
