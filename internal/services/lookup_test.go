package services

import (
	"strings"
	"testing"
)

func TestLookupAny_BinaryService(t *testing.T) {
	kind, binSvc, docSvc, err := LookupAny("mail")
	if err != nil {
		t.Fatalf("LookupAny(\"mail\") error = %v", err)
	}
	if kind != KindBinary {
		t.Errorf("kind = %v, want KindBinary", kind)
	}
	if binSvc == nil {
		t.Error("binSvc is nil; want non-nil for binary kind")
	}
	if docSvc != nil {
		t.Errorf("docSvc = %#v, want nil for binary kind", docSvc)
	}
}

func TestLookupAny_DockerService(t *testing.T) {
	kind, binSvc, docSvc, err := LookupAny("redis")
	if err != nil {
		t.Fatalf("LookupAny(\"redis\") error = %v", err)
	}
	if kind != KindDocker {
		t.Errorf("kind = %v, want KindDocker", kind)
	}
	if docSvc == nil {
		t.Error("docSvc is nil; want non-nil for docker kind")
	}
	if binSvc != nil {
		t.Errorf("binSvc = %#v, want nil for docker kind", binSvc)
	}
}

func TestLookupAny_Unknown(t *testing.T) {
	kind, binSvc, docSvc, err := LookupAny("mongodb")
	if err == nil {
		t.Fatal("LookupAny(\"mongodb\") error = nil; want non-nil for unknown name")
	}
	if kind != KindUnknown {
		t.Errorf("kind = %v, want KindUnknown", kind)
	}
	if binSvc != nil || docSvc != nil {
		t.Errorf("binSvc=%v docSvc=%v; want both nil", binSvc, docSvc)
	}
	if !strings.Contains(err.Error(), `unknown service "mongodb"`) {
		t.Errorf("error %q missing expected text", err)
	}
	if !strings.Contains(err.Error(), "available:") {
		t.Errorf("error %q missing available list", err)
	}
}

func TestLookupAny_BinaryWinsOnCollision(t *testing.T) {
	// Pin the lookup-order invariant by temporarily seeding both registries
	// with the same key. Restore via t.Cleanup so other tests are unaffected.
	const key = "collisiontest"

	// Stash any pre-existing entries (defensive — there should be none).
	prevBin, hadBin := binaryRegistry[key]
	prevDoc, hadDoc := registry[key]
	t.Cleanup(func() {
		if hadBin {
			binaryRegistry[key] = prevBin
		} else {
			delete(binaryRegistry, key)
		}
		if hadDoc {
			registry[key] = prevDoc
		} else {
			delete(registry, key)
		}
	})

	binaryRegistry[key] = &Mailpit{} // any BinaryService will do
	registry[key] = &Redis{}         // any Service will do

	kind, binSvc, docSvc, err := LookupAny(key)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if kind != KindBinary {
		t.Errorf("kind = %v, want KindBinary (binary should win on collision)", kind)
	}
	if binSvc == nil {
		t.Error("binSvc is nil; want non-nil")
	}
	if docSvc != nil {
		t.Errorf("docSvc = %#v, want nil (binary won)", docSvc)
	}
}
