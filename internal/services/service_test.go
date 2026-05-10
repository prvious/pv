package services

import (
	"testing"
)

func TestLookup_Invalid(t *testing.T) {
	_, err := Lookup("mongodb")
	if err == nil {
		t.Error("expected error for unknown service, got nil")
	}
}

func TestLookup_BinaryService(t *testing.T) {
	svc, err := Lookup("mail")
	if err != nil {
		t.Fatalf("Lookup(\"mail\") error = %v", err)
	}
	if svc == nil {
		t.Error("Lookup(\"mail\") returned nil service")
	}
}

func TestServiceKey(t *testing.T) {
	tests := []struct {
		name, version, want string
	}{
		{"mysql", "8.0.32", "mysql:8.0.32"},
		{"mysql", "latest", "mysql"},
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
	// 2 binary services: s3, mail.
	if len(names) != 2 {
		t.Errorf("Available() returned %d services, want 2", len(names))
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
