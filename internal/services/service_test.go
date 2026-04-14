package services

import (
	"testing"
)

func TestLookup_Valid(t *testing.T) {
	// s3 is now a BinaryService (RustFS), not in the Docker registry.
	for _, name := range []string{"mail", "mysql", "postgres", "redis"} {
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
	// 4 Docker services (mail, mysql, postgres, redis) + 1 binary service (s3 via RustFS).
	if len(names) != 5 {
		t.Errorf("Available() returned %d services, want 5", len(names))
	}
}

func TestParseServiceKey(t *testing.T) {
	tests := []struct {
		key         string
		wantName    string
		wantVersion string
	}{
		{"mysql:8.4", "mysql", "8.4"},
		{"mysql:8.0.32", "mysql", "8.0.32"},
		{"postgres:18-alpine", "postgres", "18-alpine"},
		{"redis", "redis", "latest"},
		{"s3", "s3", "latest"},
		{":8.4", ":8.4", "latest"}, // edge: no name before colon, idx == 0
	}
	for _, tt := range tests {
		name, version := ParseServiceKey(tt.key)
		if name != tt.wantName || version != tt.wantVersion {
			t.Errorf("ParseServiceKey(%q) = (%q, %q), want (%q, %q)",
				tt.key, name, version, tt.wantName, tt.wantVersion)
		}
	}
}

func TestSanitizeProjectName(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{"my-app", "my_app"},
		{"simple", "simple"},
		{"my_app_123", "my_app_123"},
		{"test'; DROP TABLE--", "testDROPTABLE__"},
		{"my`app", "myapp"},
		{"hello world", "helloworld"},
		{"café", "caf"},
		{"a-b.c/d@e", "a_bcde"},
	}
	for _, tt := range tests {
		got := SanitizeProjectName(tt.input)
		if got != tt.want {
			t.Errorf("SanitizeProjectName(%q) = %q, want %q", tt.input, got, tt.want)
		}
	}
}
