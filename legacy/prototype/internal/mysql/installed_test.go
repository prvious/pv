package mysql

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestInstalledVersions_Empty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	got, err := InstalledVersions()
	if err != nil {
		t.Fatalf("InstalledVersions: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("expected empty, got %v", got)
	}
}

func TestInstalledVersions_FindsBinaries(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	for _, version := range []string{"8.0", "8.4", "9.7"} {
		bin := config.MysqlBinDir(version)
		if err := os.MkdirAll(bin, 0o755); err != nil {
			t.Fatalf("mkdir: %v", err)
		}
		if err := os.WriteFile(filepath.Join(bin, "mysqld"), []byte("#!/bin/sh\n"), 0o755); err != nil {
			t.Fatalf("write: %v", err)
		}
	}
	got, err := InstalledVersions()
	if err != nil {
		t.Fatalf("InstalledVersions: %v", err)
	}
	sort.Strings(got)
	want := []string{"8.0", "8.4", "9.7"}
	if len(got) != 3 {
		t.Fatalf("InstalledVersions = %v, want %v", got, want)
	}
	for i := range got {
		if got[i] != want[i] {
			t.Errorf("InstalledVersions[%d] = %q, want %q", i, got[i], want[i])
		}
	}
}

func TestInstalledVersions_DirWithoutBinary_NotInstalled(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	if err := os.MkdirAll(config.MysqlVersionDir("8.4"), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	got, err := InstalledVersions()
	if err != nil {
		t.Fatalf("InstalledVersions: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("dir without bin/mysqld should not count: got %v", got)
	}
}

func TestIsInstalled(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if IsInstalled("8.4") {
		t.Error("IsInstalled should be false on empty home")
	}
	bin := config.MysqlBinDir("8.4")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(bin, "mysqld"), []byte{}, 0o755); err != nil {
		t.Fatalf("write: %v", err)
	}
	if !IsInstalled("8.4") {
		t.Error("IsInstalled should be true after writing bin/mysqld")
	}
}
