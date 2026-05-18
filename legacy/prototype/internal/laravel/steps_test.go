package laravel

import (
	"testing"
)

// --- isLaravel tests ---

func TestIsLaravel(t *testing.T) {
	tests := []struct {
		input string
		want  bool
	}{
		{"laravel", true},
		{"laravel-octane", true},
		{"php", false},
		{"static", false},
		{"", false},
	}
	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			if got := isLaravel(tt.input); got != tt.want {
				t.Errorf("isLaravel(%q) = %v, want %v", tt.input, got, tt.want)
			}
		})
	}
}
