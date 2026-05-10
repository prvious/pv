package mailpit

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
)

func TestSetEnabled_NotRegistered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	err := SetEnabled(true)
	if err == nil {
		t.Fatal("expected error when service is not registered")
	}
	msg := err.Error()
	if !strings.Contains(msg, "not registered") || !strings.Contains(msg, "mailpit:install") {
		t.Errorf("error should point user at mailpit:install; got %q", msg)
	}
}

func TestUpdate_NotRegistered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	err := Update()
	if err == nil {
		t.Fatal("expected error when service is not registered")
	}
	if !strings.Contains(err.Error(), "not registered") {
		t.Errorf("expected not-registered error; got %q", err)
	}
}

func TestUninstall_NotRegistered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	err := Uninstall(false)
	if err == nil {
		t.Fatal("expected error when service is not registered")
	}
	if !strings.Contains(err.Error(), "not registered") {
		t.Errorf("expected not-registered error; got %q", err)
	}
}

// TestUninstall_BinaryAlreadyRemoved verifies that an idempotent retry
// after a previous run that left the registry intact but removed the
// binary file completes successfully. This is the recoverable state
// the step ordering (registry-removed-last) is designed to enable.
func TestUninstall_BinaryAlreadyRemoved(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Port: 1025, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Uninstall(false); err != nil {
		t.Fatalf("Uninstall with no binary file should succeed: %v", err)
	}
	got, err := registry.Load()
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	if _, ok := got.Services["mail"]; ok {
		t.Error("registry entry should be removed after successful uninstall")
	}
}

// TestUninstall_DeleteData verifies that --force/data-deletion actually
// wipes the data directory. This is the irreversible postgres-style
// :uninstall semantic; a regression here would silently spare user data
// the user explicitly asked to be deleted.
func TestUninstall_DeleteData(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dataDir := config.ServiceDataDir("mail", "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "mailpit.db")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Port: 1025, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Uninstall(true); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); !os.IsNotExist(err) {
		t.Errorf("data directory must be deleted; sentinel still exists (err=%v)", err)
	}
}

// TestRestart_RoundTrip locks the contract that after Restart the
// registry shows Enabled=true: the disable/enable toggle must persist
// the second flip.
func TestRestart_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Port: 1025, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Restart(); err != nil {
		t.Fatalf("Restart: %v", err)
	}

	got, err := registry.Load()
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	inst, ok := got.Services["mail"]
	if !ok {
		t.Fatal("mail entry missing from registry after Restart")
	}
	if inst.Enabled == nil || !*inst.Enabled {
		t.Errorf("Enabled should be true after Restart; got %v", inst.Enabled)
	}
}
