package redis

import (
	"testing"
)

func TestState_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Empty home → empty state.
	s, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if s.Wanted != "" {
		t.Errorf("LoadState on empty home: Wanted = %q, want empty", s.Wanted)
	}

	if err := SetWanted(WantedRunning); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}

	s, err = LoadState()
	if err != nil {
		t.Fatalf("LoadState after SetWanted: %v", err)
	}
	if s.Wanted != WantedRunning {
		t.Errorf("LoadState.Wanted = %q, want %q", s.Wanted, WantedRunning)
	}
}

func TestSetWanted_InvalidRejected(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("not-a-real-state"); err == nil {
		t.Error("SetWanted should reject unknown values")
	}
}

func TestRemoveState(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted(WantedRunning); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}
	if err := RemoveState(); err != nil {
		t.Fatalf("RemoveState: %v", err)
	}
	s, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState after RemoveState: %v", err)
	}
	if s.Wanted != "" {
		t.Errorf("LoadState.Wanted after RemoveState = %q, want empty", s.Wanted)
	}
}
