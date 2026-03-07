package phpenv

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestWriteShims_CreatesPhpShim(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	if err := WriteShims(); err != nil {
		t.Fatalf("WriteShims() error = %v", err)
	}

	shimPath := filepath.Join(config.BinDir(), "php")
	info, err := os.Stat(shimPath)
	if err != nil {
		t.Fatalf("php shim not created: %v", err)
	}
	if info.Mode()&0111 == 0 {
		t.Error("php shim is not executable")
	}

	content, _ := os.ReadFile(shimPath)
	if !strings.Contains(string(content), "#!/bin/bash") {
		t.Error("php shim missing shebang")
	}
	if !strings.Contains(string(content), config.PhpDir()) {
		t.Error("php shim missing PHP dir path")
	}
}

func TestWriteShims_CreatesComposerSymlink(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	if err := WriteShims(); err != nil {
		t.Fatalf("WriteShims() error = %v", err)
	}

	linkPath := filepath.Join(config.BinDir(), "composer")
	target, err := os.Readlink(linkPath)
	if err != nil {
		t.Fatalf("composer symlink not created: %v", err)
	}
	if target != config.ComposerPharPath() {
		t.Errorf("composer symlink target = %q, want %q", target, config.ComposerPharPath())
	}
}
