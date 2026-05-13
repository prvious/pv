package rustfs

import (
	"net"
	"testing"
	"time"
)

func TestWaitStopped_AlreadyDown(t *testing.T) {
	// Port 9000 should be down in test env; WaitStopped returns immediately.
	if err := WaitStopped(DefaultVersion(), 2*time.Second); err != nil {
		t.Fatalf("WaitStopped when port down: %v", err)
	}
}

func TestWaitStopped_TimeoutWhenUp(t *testing.T) {
	// Bind a listener on the RustFS port so WaitStopped times out.
	ln, err := net.Listen("tcp", "127.0.0.1:9000")
	if err != nil {
		t.Skipf("cannot bind port 9000: %v", err)
	}
	defer ln.Close()

	err = WaitStopped(DefaultVersion(), 500*time.Millisecond)
	if err == nil {
		t.Fatal("WaitStopped: expected timeout error when port is up")
	}
	if !contains(err.Error(), "did not stop") {
		t.Errorf("error = %v; want 'did not stop'", err)
	}
}

func contains(s, substr string) bool {
	return len(s) >= len(substr) && (s == substr || len(s) > 0 && containsHelper(s, substr))
}

func containsHelper(s, substr string) bool {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
