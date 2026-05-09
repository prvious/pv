package svchooks

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

func mustS3(t *testing.T) services.BinaryService {
	t.Helper()
	svc, ok := services.LookupBinary("s3")
	if !ok {
		t.Fatal("s3 binary service must be registered (build issue)")
	}
	return svc
}

func mustMail(t *testing.T) services.BinaryService {
	t.Helper()
	svc, ok := services.LookupBinary("mail")
	if !ok {
		t.Fatal("mail binary service must be registered (build issue)")
	}
	return svc
}

func TestSetEnabled_NotRegistered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}

	err := SetEnabled(reg, mustS3(t), true)
	if err == nil {
		t.Fatal("expected error when service is not registered")
	}
	msg := err.Error()
	if !strings.Contains(msg, "not registered") || !strings.Contains(msg, "rustfs:install") {
		t.Errorf("error should point user at rustfs:install; got %q", msg)
	}
}

// TestSetEnabled_KindGuard locks the upgrade-guard contract: a docker-shaped
// "s3" entry from a pv version that predated the binary-service migration
// must not flip Enabled and pretend to start something — the daemon
// doesn't supervise docker entries.
func TestSetEnabled_KindGuard(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "", Image: "rustfs/rustfs:latest", Port: 9000},
		},
	}

	err := SetEnabled(reg, mustS3(t), true)
	if err == nil {
		t.Fatal("expected error for legacy docker-shaped s3 entry")
	}
	if !strings.Contains(err.Error(), "previous pv version") {
		t.Errorf("error should mention upgrade path; got %q", err)
	}
	// Must not have mutated Enabled.
	if reg.Services["s3"].Enabled != nil {
		t.Error("Enabled must not be set on a docker-shaped entry")
	}
}

func TestUpdate_NotRegistered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}

	err := Update(reg, mustS3(t))
	if err == nil {
		t.Fatal("expected error when service is not registered")
	}
	if !strings.Contains(err.Error(), "not registered") {
		t.Errorf("expected not-registered error; got %q", err)
	}
}

func TestUpdate_KindGuard(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Kind: "docker", Image: "axllent/mailpit:latest", Port: 1025},
		},
	}

	err := Update(reg, mustMail(t))
	if err == nil {
		t.Fatal("expected error for legacy docker-shaped mail entry")
	}
	if !strings.Contains(err.Error(), "previous pv version") {
		t.Errorf("error should mention upgrade path; got %q", err)
	}
}

// TestInstall_KindGuard replaces the resolveKind upgrade-guard test that
// was deleted with resolveKind itself. The kind check now lives inside
// svchooks.Install — a docker-shaped registry entry from a previous pv
// version must produce a redirect-to-uninstall error rather than
// silently being treated as "already added".
func TestInstall_KindGuard(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "", Image: "rustfs/rustfs:latest", Port: 9000},
		},
	}

	err := Install(reg, mustS3(t))
	if err == nil {
		t.Fatal("expected error for docker-shaped s3 entry on Install")
	}
	if !strings.Contains(err.Error(), "previous pv version") {
		t.Errorf("error should mention upgrade path; got %q", err)
	}
}

func TestUninstall_NotRegistered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}

	err := Uninstall(mustS3(t), reg, false)
	if err == nil {
		t.Fatal("expected error when service is not registered")
	}
	if !strings.Contains(err.Error(), "not registered") {
		t.Errorf("expected not-registered error; got %q", err)
	}
}

func TestUninstall_KindGuard(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "docker", Image: "rustfs/rustfs:latest", Port: 9000},
		},
	}

	err := Uninstall(mustS3(t), reg, true)
	if err == nil {
		t.Fatal("expected error for docker-shaped s3 on uninstall — must not silently delete files")
	}
	// Registry entry must be untouched so the user can still resolve the
	// state via `pv uninstall && pv setup` per the error guidance.
	if _, ok := reg.Services["s3"]; !ok {
		t.Error("registry entry was removed despite kind-guard failure")
	}
}

// TestUninstall_BinaryAlreadyRemoved verifies that an idempotent retry
// after a previous run that left the registry intact but removed the
// binary file completes successfully. This is the recoverable state the
// new step ordering (registry-removed-last) is designed to enable.
func TestUninstall_BinaryAlreadyRemoved(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "binary", Port: 9000, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	// No binary file present at config.InternalBinDir() — Uninstall must
	// tolerate the missing file and complete cleanly.
	if err := Uninstall(mustS3(t), reg, false); err != nil {
		t.Fatalf("Uninstall with no binary file should succeed: %v", err)
	}
	if _, ok := reg.Services["s3"]; ok {
		t.Error("registry entry should be removed after successful uninstall")
	}
}

// TestUninstall_DeleteData verifies that --force/data-deletion actually
// wipes the data directory. This is the irreversible postgres-style
// :uninstall semantic; a regression here would silently spare user data
// the user explicitly asked to be deleted.
func TestUninstall_DeleteData(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)

	dataDir := config.ServiceDataDir("s3", "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	sentinel := filepath.Join(dataDir, "buckets.json")
	if err := os.WriteFile(sentinel, []byte("{}"), 0o644); err != nil {
		t.Fatalf("write sentinel: %v", err)
	}

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "binary", Port: 9000, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Uninstall(mustS3(t), reg, true); err != nil {
		t.Fatalf("Uninstall: %v", err)
	}
	if _, err := os.Stat(sentinel); !os.IsNotExist(err) {
		t.Errorf("data directory must be deleted; sentinel still exists (err=%v)", err)
	}
}

// TestRestart_RoundTrip locks the contract that after Restart the
// registry shows Enabled=true: the disable/enable toggle must persist
// the second flip. A regression that drops the registry reload between
// the two SetEnabled calls would silently leave the service disabled.
func TestRestart_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "binary", Port: 9000, Enabled: &enabled},
		},
	}
	if err := reg.Save(); err != nil {
		t.Fatalf("save: %v", err)
	}

	if err := Restart(reg, mustS3(t)); err != nil {
		t.Fatalf("Restart: %v", err)
	}

	// Reload from disk to confirm the persisted state, since Restart's
	// in-memory pointer was rewritten between calls.
	got, err := registry.Load()
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	inst, ok := got.Services["s3"]
	if !ok {
		t.Fatal("s3 entry missing from registry after Restart")
	}
	if inst.Enabled == nil || !*inst.Enabled {
		t.Errorf("Enabled should be true after Restart; got %v", inst.Enabled)
	}
}
