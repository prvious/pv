package mysql

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func installFakeMysqlVersion(t *testing.T, version string) {
	t.Helper()
	bin := config.MysqlBinDir(version)
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(bin, "mysqld"), []byte{}, 0o755); err != nil {
		t.Fatalf("write: %v", err)
	}
}

func TestWantedVersions_Intersection(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	installFakeMysqlVersion(t, "8.4")
	installFakeMysqlVersion(t, "9.7")
	_ = SetWanted("8.4", WantedRunning)
	_ = SetWanted("9.7", WantedStopped)
	got, err := WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions: %v", err)
	}
	if len(got) != 1 || got[0] != "8.4" {
		t.Errorf("WantedVersions = %v, want [8.4]", got)
	}
}

func TestWantedVersions_StaleStateFiltered(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// state says running but never installed
	_ = SetWanted("8.4", WantedRunning)
	got, err := WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("stale state should be filtered, got %v", got)
	}
}

func TestWantedVersions_SortedOutput(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	for _, v := range []string{"9.7", "8.0", "8.4"} {
		installFakeMysqlVersion(t, v)
		_ = SetWanted(v, WantedRunning)
	}
	got, err := WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions: %v", err)
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

func TestWantedVersions_NoStateNoVersions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	got, err := WantedVersions()
	if err != nil {
		t.Fatalf("WantedVersions: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("empty home should yield no wanted versions, got %v", got)
	}
}
