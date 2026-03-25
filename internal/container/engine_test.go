package container

import "testing"

func TestNewEngine_StoresSocketPath(t *testing.T) {
	path := "/tmp/test-docker.sock"
	engine, err := NewEngine(path)
	if err != nil {
		t.Fatalf("NewEngine() error = %v", err)
	}
	defer engine.Close()

	if got := engine.SocketPath(); got != path {
		t.Errorf("SocketPath() = %q, want %q", got, path)
	}
}

func TestNewEngine_InvalidSocket(t *testing.T) {
	// Docker client creation is lazy — it should not error even with a
	// nonexistent socket path. The error only surfaces on actual API calls.
	engine, err := NewEngine("/nonexistent/path/docker.sock")
	if err != nil {
		t.Fatalf("NewEngine() error = %v, want nil (connection is lazy)", err)
	}
	defer engine.Close()
}

func TestNewEngine_CloseNil(t *testing.T) {
	engine, err := NewEngine("/tmp/test-docker.sock")
	if err != nil {
		t.Fatalf("NewEngine() error = %v", err)
	}

	// Close should not panic on a valid engine.
	if err := engine.Close(); err != nil {
		t.Errorf("Close() error = %v, want nil", err)
	}
}
