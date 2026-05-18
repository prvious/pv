package projectenv

import (
	"testing"
)

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
