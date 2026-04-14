package service

import (
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
