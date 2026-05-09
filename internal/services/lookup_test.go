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

func TestLookupAny_RedisRemoved(t *testing.T) {
	// After redis migrated to a native binary, the docker registry is empty.
	// LookupAny("redis") must now return the unknown-service error path.
	_, _, _, err := LookupAny("redis")
	if err == nil {
		t.Error("LookupAny(\"redis\") should error after docker redis removal")
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

// Note: TestLookupAny_BinaryWinsOnCollision was dropped when redis (the
// last docker Service) migrated to a native binary. With the docker
// registry empty there's no real Service implementation left to seed
// the test with; collision behavior is now unobservable without
// constructing a synthetic Service stub purely for the test, which adds
// more surface area than the invariant is worth. If a docker Service is
// ever reintroduced, restore the collision test against it.
