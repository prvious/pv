//go:build darwin

package colima

import "testing"

func TestParseMajorVersion(t *testing.T) {
	tests := []struct {
		input string
		want  int
	}{
		{"14.5", 14},
		{"13.0.1", 13},
		{"15", 15},
		{"12.6.7", 12},
	}

	for _, tt := range tests {
		got, err := parseMajorVersion(tt.input)
		if err != nil {
			t.Errorf("parseMajorVersion(%q) error: %v", tt.input, err)
			continue
		}
		if got != tt.want {
			t.Errorf("parseMajorVersion(%q) = %d, want %d", tt.input, got, tt.want)
		}
	}
}

func TestCheckVZCompat_CurrentMachine(t *testing.T) {
	// On any macOS CI or dev machine running this test, the check should pass
	// (we require 13+ and all modern dev/CI machines meet this).
	err := checkVZCompat()
	if err != nil {
		t.Logf("checkVZCompat returned: %v (expected on older macOS)", err)
	}
}
