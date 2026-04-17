package service

import (
	"strings"
	"testing"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

func TestResolveKind_BinaryServiceByName(t *testing.T) {
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	kind, bin, doc, err := resolveKind(reg, "s3")
	if err != nil {
		t.Fatalf("resolveKind: %v", err)
	}
	if kind != kindBinary {
		t.Errorf("kind = %v, want kindBinary", kind)
	}
	if bin == nil {
		t.Error("binary service should be non-nil")
	}
	if doc != nil {
		t.Error("docker service should be nil")
	}
	if _, ok := bin.(*services.RustFS); !ok {
		t.Errorf("expected *RustFS, got %T", bin)
	}
}

func TestResolveKind_DockerServiceByName(t *testing.T) {
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	kind, bin, doc, err := resolveKind(reg, "mysql")
	if err != nil {
		t.Fatalf("resolveKind: %v", err)
	}
	if kind != kindDocker {
		t.Errorf("kind = %v, want kindDocker", kind)
	}
	if doc == nil {
		t.Error("docker service should be non-nil")
	}
	if bin != nil {
		t.Error("binary service should be nil")
	}
}

func TestResolveKind_Unknown_ReturnsError(t *testing.T) {
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	_, _, _, err := resolveKind(reg, "bogus")
	if err == nil {
		t.Fatal("expected error for unknown service")
	}
}

func TestResolveKind_DockerEntryBlocksBinaryRegistration(t *testing.T) {
	// Pre-existing Docker "s3" entry (from older pv) should error on a
	// service:add for the now-binary "s3" — no silent auto-migration.
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "", Image: "rustfs/rustfs:latest", Port: 9000},
		},
	}
	_, _, _, err := resolveKind(reg, "s3")
	if err == nil {
		t.Fatal("expected error for pre-existing docker s3 entry")
	}
}

func TestResolveKind_MailDockerEntryBlocksBinaryMigration(t *testing.T) {
	// Same scenario as the s3 test above but for mail — the more common
	// upgrade path since users who ran `pv service:add mail` on an older
	// version will have a docker-shaped entry.
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"mail": {Kind: "", Image: "axllent/mailpit:latest", Port: 1025},
		},
	}
	_, _, _, err := resolveKind(reg, "mail")
	if err == nil {
		t.Fatal("expected error for pre-existing docker mail entry")
	}
	if !strings.Contains(err.Error(), "pv uninstall") {
		t.Errorf("error should mention remedy; got %q", err)
	}
}

func TestResolveKind_MailBinaryService(t *testing.T) {
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	kind, bin, doc, err := resolveKind(reg, "mail")
	if err != nil {
		t.Fatalf("resolveKind: %v", err)
	}
	if kind != kindBinary {
		t.Errorf("kind = %v, want kindBinary", kind)
	}
	if bin == nil {
		t.Error("binary service should be non-nil")
	}
	if doc != nil {
		t.Error("docker service should be nil")
	}
	if _, ok := bin.(*services.Mailpit); !ok {
		t.Errorf("expected *Mailpit, got %T", bin)
	}
}
