package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestWantedVersions_Empty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	vs, err := WantedVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 0 {
		t.Error("expected empty")
	}
}

func TestWantedVersions_WantedAndInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := SetWanted("8.6", WantedRunning); err != nil {
		t.Fatal(err)
	}

	versionDir := config.RedisVersionDir("8.6")
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)

	vs, err := WantedVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 1 || vs[0] != "8.6" {
		t.Errorf("got %v, want [8.6]", vs)
	}
}

func TestWantedVersions_WantedButNotInstalled_Skipped(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	if err := SetWanted("8.6", WantedRunning); err != nil {
		t.Fatal(err)
	}

	vs, err := WantedVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 0 {
		t.Error("expected empty when binary missing")
	}
}
