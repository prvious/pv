package redis

import "testing"

func TestPortFor(t *testing.T) {
	if got := PortFor(); got != 6379 {
		t.Errorf("PortFor() = %d, want 6379", got)
	}
}
