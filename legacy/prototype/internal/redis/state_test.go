package redis

import "testing"

func TestLoadState_MissingReturnsEmptyVersions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s, err := LoadState()
	if err != nil {
		t.Fatal(err)
	}
	if s.Versions == nil {
		t.Error("Versions map should not be nil")
	}
	if len(s.Versions) != 0 {
		t.Error("expected empty versions")
	}
}

func TestSetWanted_Roundtrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("8.6", WantedRunning); err != nil {
		t.Fatal(err)
	}
	s, err := LoadState()
	if err != nil {
		t.Fatal(err)
	}
	if s.Versions["8.6"].Wanted != WantedRunning {
		t.Errorf("wanted = %q", s.Versions["8.6"].Wanted)
	}
}

func TestSetWanted_RejectsInvalid(t *testing.T) {
	if err := SetWanted("8.6", "invalid"); err == nil {
		t.Error("expected error for invalid wanted state")
	}
}

func TestRemoveVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	SetWanted("8.6", WantedRunning)
	if err := RemoveVersion("8.6"); err != nil {
		t.Fatal(err)
	}
	s, err := LoadState()
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := s.Versions["8.6"]; ok {
		t.Error("version should be removed")
	}
}

func TestRemoveState(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	SetWanted("8.6", WantedRunning)
	if err := RemoveState(); err != nil {
		t.Fatal(err)
	}
	s, err := LoadState()
	if err != nil {
		t.Fatal(err)
	}
	if len(s.Versions) != 0 {
		t.Error("expected empty after RemoveState")
	}
}
