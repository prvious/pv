package registry

import (
	"testing"
)

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
