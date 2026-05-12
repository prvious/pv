package redis

import "testing"

func TestPortFor(t *testing.T) {
	tests := []struct {
		version string
		want    int
	}{
		{"7.4", 6740},
		{"8.6", 6860},
	}
	for _, tc := range tests {
		got := PortFor(tc.version)
		if got != tc.want {
			t.Errorf("PortFor(%q) = %d, want %d", tc.version, got, tc.want)
		}
	}
}
