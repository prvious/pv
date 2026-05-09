package redis

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestIsInstalled_Empty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if IsInstalled() {
		t.Error("IsInstalled should be false on empty home")
	}
}

func TestIsInstalled_FindsBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	bin := filepath.Join(config.RedisDir(), "redis-server")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatalf("write: %v", err)
	}
	if !IsInstalled() {
		t.Error("IsInstalled should be true after writing redis-server")
	}
}

func TestIsInstalled_DirWithoutBinary(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	if err := os.MkdirAll(config.RedisDir(), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if IsInstalled() {
		t.Error("dir without redis-server should not count as installed")
	}
}
