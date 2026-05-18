package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestServerBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	path := ServerBinary("8.6")
	if path == "" {
		t.Error("expected non-empty path")
	}
}

func TestIsInstalled_True(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	versionDir := config.RedisVersionDir("8.6")
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)
	if !IsInstalled("8.6") {
		t.Error("expected installed")
	}
}

func TestIsInstalled_False(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if IsInstalled("8.6") {
		t.Error("expected not installed")
	}
}

func TestInstalledVersions(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	vs, err := InstalledVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 0 {
		t.Error("expected no versions")
	}

	versionDir := config.RedisVersionDir("8.6")
	os.MkdirAll(versionDir, 0o755)
	os.WriteFile(filepath.Join(versionDir, "redis-server"), []byte("x"), 0o755)

	vs, err = InstalledVersions()
	if err != nil {
		t.Fatal(err)
	}
	if len(vs) != 1 || vs[0] != "8.6" {
		t.Errorf("got %v, want [8.6]", vs)
	}
}
