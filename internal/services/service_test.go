package services

import (
	"testing"
)

func TestLookup_Valid(t *testing.T) {
	for _, name := range []string{"mysql", "postgres", "redis", "rustfs"} {
		svc, err := Lookup(name)
		if err != nil {
			t.Errorf("Lookup(%q) error = %v", name, err)
		}
		if svc.Name() != name {
			t.Errorf("Lookup(%q).Name() = %q", name, svc.Name())
		}
	}
}

func TestLookup_Invalid(t *testing.T) {
	_, err := Lookup("mongodb")
	if err == nil {
		t.Error("expected error for unknown service, got nil")
	}
}

func TestServiceKey(t *testing.T) {
	tests := []struct {
		name, version, want string
	}{
		{"mysql", "8.0.32", "mysql:8.0.32"},
		{"mysql", "latest", "mysql"},
		{"redis", "", "redis"},
		{"redis", "latest", "redis"},
		{"postgres", "16", "postgres:16"},
	}
	for _, tt := range tests {
		got := ServiceKey(tt.name, tt.version)
		if got != tt.want {
			t.Errorf("ServiceKey(%q, %q) = %q, want %q", tt.name, tt.version, got, tt.want)
		}
	}
}

func TestAvailable(t *testing.T) {
	names := Available()
	if len(names) != 4 {
		t.Errorf("Available() returned %d services, want 4", len(names))
	}
}
