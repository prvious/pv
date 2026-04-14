package services

import "testing"

func TestLookupBinary_Unknown_ReturnsFalse(t *testing.T) {
	_, ok := LookupBinary("does-not-exist")
	if ok {
		t.Error("expected ok=false for unknown name")
	}
}

func TestLookupBinary_KnownRegistered(t *testing.T) {
	// This test is populated by Task 5 when RustFS is registered.
	// For now we just assert the function exists and returns the empty-map result.
	if binaryRegistry == nil {
		t.Fatal("binaryRegistry should not be nil")
	}
}

func TestAllBinary_ReturnsRegistryMap(t *testing.T) {
	m := AllBinary()
	if m == nil {
		t.Error("AllBinary should not return nil")
	}
	// Identity equality is not guaranteed by the interface, but content equality is.
	if len(m) != len(binaryRegistry) {
		t.Errorf("AllBinary size %d != binaryRegistry size %d", len(m), len(binaryRegistry))
	}
}
