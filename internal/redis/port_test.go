package redis

import "testing"

func TestPortFor(t *testing.T) {
	tests := []struct {
		version string
		want    int
	}{
		{"7.4", 7040},
		{"8.6", 7160},
	}
	for _, tc := range tests {
		got := PortFor(tc.version)
		if got != tc.want {
			t.Errorf("PortFor(%q) = %d, want %d", tc.version, got, tc.want)
		}
	}
}
