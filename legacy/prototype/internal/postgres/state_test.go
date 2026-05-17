package postgres

import "testing"

func TestState_DefaultEmpty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if len(st.Majors) != 0 {
		t.Errorf("expected empty, got %d", len(st.Majors))
	}
}

func TestState_SetAndPersist(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("17", "running"); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if got := st.Majors["17"].Wanted; got != "running" {
		t.Errorf("Wanted = %q, want running", got)
	}
}

func TestState_RemoveMajor(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	_ = SetWanted("17", "running")
	_ = SetWanted("18", "stopped")
	if err := RemoveMajor("17"); err != nil {
		t.Fatalf("RemoveMajor: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if _, ok := st.Majors["17"]; ok {
		t.Error("17 should be removed")
	}
	if _, ok := st.Majors["18"]; !ok {
		t.Error("18 should still be present")
	}
}
