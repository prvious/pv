package mysql

import "testing"

func TestPortFor(t *testing.T) {
	tests := []struct {
		version string
		want    int
	}{
		{"8.0", 33080},
		{"8.4", 33084},
		{"9.7", 33097},
		// Unconstrained but parsable — PortFor doesn't gate on the
		// supported-version allow-list (callers do that).
		{"10.0", 33100},
	}
	for _, tt := range tests {
		got, err := PortFor(tt.version)
		if err != nil {
			t.Errorf("PortFor(%q): %v", tt.version, err)
			continue
		}
		if got != tt.want {
			t.Errorf("PortFor(%q) = %d, want %d", tt.version, got, tt.want)
		}
	}
}

func TestPortFor_Invalid(t *testing.T) {
	for _, v := range []string{"", "8", "8.x", "8.4.1", "abc", "-1.0", "8.-1"} {
		if _, err := PortFor(v); err == nil {
			t.Errorf("PortFor(%q) should error", v)
		}
	}
	if _, err := PortFor("100.0"); err == nil {
		t.Error("PortFor major > 99 should error (would overflow port range)")
	}
	if _, err := PortFor("1.100"); err == nil {
		t.Error("PortFor minor > 99 should error")
	}
}
