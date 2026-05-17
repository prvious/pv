package postgres

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func installFakeMajor(t *testing.T, major string) {
	t.Helper()
	bin := config.PostgresBinDir(major)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte{}, 0o755); err != nil {
		t.Fatalf("write: %v", err)
	}
}

func TestWantedMajors_Intersection(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	installFakeMajor(t, "17")
	installFakeMajor(t, "18")
	_ = SetWanted("17", "running")
	_ = SetWanted("18", "stopped")
	got, err := WantedMajors()
	if err != nil {
		t.Fatalf("WantedMajors: %v", err)
	}
	if len(got) != 1 || got[0] != "17" {
		t.Errorf("WantedMajors = %v, want [17]", got)
	}
}

func TestWantedMajors_StaleStateFiltered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	_ = SetWanted("17", "running")
	got, err := WantedMajors()
	if err != nil {
		t.Fatalf("WantedMajors: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("stale state should be filtered, got %v", got)
	}
}

func TestWantedMajors_SortedOutput(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	for _, m := range []string{"18", "17"} {
		installFakeMajor(t, m)
		_ = SetWanted(m, "running")
	}
	got, err := WantedMajors()
	if err != nil {
		t.Fatalf("WantedMajors: %v", err)
	}
	sorted := make([]string, len(got))
	copy(sorted, got)
	sort.Strings(sorted)
	for i := range got {
		if got[i] != sorted[i] {
			t.Errorf("output not sorted: %v", got)
			break
		}
	}
}
