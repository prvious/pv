package postgres

import (
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestInstalledMajors_Empty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	got, err := InstalledMajors()
	if err != nil {
		t.Fatalf("InstalledMajors: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("expected empty, got %v", got)
	}
}

func TestInstalledMajors_FindsBinaries(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	for _, major := range []string{"17", "18"} {
		bin := filepath.Join(config.PostgresBinDir(major))
		if err := os.MkdirAll(bin, 0o755); err != nil {
			t.Fatalf("mkdir: %v", err)
		}
		if err := os.WriteFile(filepath.Join(bin, "postgres"), []byte("#!/bin/sh\n"), 0o755); err != nil {
			t.Fatalf("write: %v", err)
		}
	}
	got, err := InstalledMajors()
	if err != nil {
		t.Fatalf("InstalledMajors: %v", err)
	}
	sort.Strings(got)
	want := []string{"17", "18"}
	if len(got) != 2 || got[0] != want[0] || got[1] != want[1] {
		t.Errorf("InstalledMajors = %v, want %v", got, want)
	}
}

func TestInstalledMajors_DirWithoutBinary_NotInstalled(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	if err := os.MkdirAll(config.PostgresVersionDir("17"), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	got, err := InstalledMajors()
	if err != nil {
		t.Fatalf("InstalledMajors: %v", err)
	}
	if len(got) != 0 {
		t.Errorf("dir without bin/postgres should not count: got %v", got)
	}
}
