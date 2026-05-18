package supervisor

import (
	"strings"
	"testing"
	"time"
)

func TestBuildReadyFunc_RejectsZeroValue(t *testing.T) {
	// Zero-value ReadyCheck has neither tcpPort nor httpEndpoint set.
	// The "both set" case is now unconstructable from outside the supervisor
	// package (fields are unexported) — the type system prevents it.
	_, err := BuildReadyFunc(ReadyCheck{})
	if err == nil {
		t.Fatal("expected error for zero-value ReadyCheck")
	}
	if !strings.Contains(err.Error(), "exactly one") {
		t.Errorf("error should mention 'exactly one'; got %v", err)
	}
}

func TestBuildReadyFunc_TCPOnly(t *testing.T) {
	fn, err := BuildReadyFunc(TCPReady(9000, 30*time.Second))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if fn == nil {
		t.Fatal("expected non-nil ready func")
	}
}

func TestBuildReadyFunc_HTTPOnly(t *testing.T) {
	fn, err := BuildReadyFunc(HTTPReady("http://127.0.0.1:9000/health", 30*time.Second))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if fn == nil {
		t.Fatal("expected non-nil ready func")
	}
}
