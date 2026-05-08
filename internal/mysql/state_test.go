package mysql

import (
	"testing"

	"github.com/prvious/pv/internal/state"
)

func TestState_DefaultEmpty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if len(st.Versions) != 0 {
		t.Errorf("expected empty, got %d", len(st.Versions))
	}
}

func TestState_SetAndPersist(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("8.4", WantedRunning); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if got := st.Versions["8.4"].Wanted; got != "running" {
		t.Errorf("Wanted = %q, want running", got)
	}
}

func TestState_RejectsInvalidWanted(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := SetWanted("8.4", "garbage"); err == nil {
		t.Error("SetWanted should reject unknown wanted state")
	}
}

func TestState_RemoveVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	_ = SetWanted("8.4", WantedRunning)
	_ = SetWanted("9.7", WantedStopped)
	if err := RemoveVersion("8.4"); err != nil {
		t.Fatalf("RemoveVersion: %v", err)
	}
	st, err := LoadState()
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if _, ok := st.Versions["8.4"]; ok {
		t.Error("8.4 should be removed")
	}
	if _, ok := st.Versions["9.7"]; !ok {
		t.Error("9.7 should still be present")
	}
}

func TestState_PreservesOtherServiceSlices(t *testing.T) {
	// The mysql wrapper must not stomp on the postgres slice when it
	// writes its own. Round-trip through the generic state package
	// to confirm.
	t.Setenv("HOME", t.TempDir())
	// Seed a fake "postgres" slice via the generic package, then write
	// mysql, then load and check both.
	{
		all, err := stateAllForTest()
		if err != nil {
			t.Fatalf("stateAllForTest: %v", err)
		}
		all["postgres"] = []byte(`{"majors":{"17":{"wanted":"running"}}}`)
		if err := stateSaveForTest(all); err != nil {
			t.Fatalf("save seed: %v", err)
		}
	}
	if err := SetWanted("8.4", WantedRunning); err != nil {
		t.Fatalf("SetWanted: %v", err)
	}
	all, err := stateAllForTest()
	if err != nil {
		t.Fatalf("stateAllForTest: %v", err)
	}
	if _, ok := all["postgres"]; !ok {
		t.Error("postgres slice was lost when mysql wrote its slice")
	}
	if _, ok := all["mysql"]; !ok {
		t.Error("mysql slice not written")
	}
}

func stateAllForTest() (state.State, error) { return state.Load() }
func stateSaveForTest(s state.State) error  { return state.Save(s) }
