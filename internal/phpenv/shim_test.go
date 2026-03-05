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

func TestWriteShims_CreatesComposerShim(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	if err := WriteShims(); err != nil {
		t.Fatalf("WriteShims() error = %v", err)
	}

	shimPath := filepath.Join(config.BinDir(), "composer")
	info, err := os.Stat(shimPath)
	if err != nil {
		t.Fatalf("composer shim not created: %v", err)
	}
	if info.Mode()&0111 == 0 {
		t.Error("composer shim is not executable")
	}

	content, _ := os.ReadFile(shimPath)
	s := string(content)

	if !strings.Contains(s, "#!/bin/bash") {
		t.Error("composer shim missing shebang")
	}
	if !strings.Contains(s, "COMPOSER_HOME=") {
		t.Error("composer shim missing COMPOSER_HOME")
	}
	if !strings.Contains(s, "COMPOSER_CACHE_DIR=") {
		t.Error("composer shim missing COMPOSER_CACHE_DIR")
	}
	if !strings.Contains(s, config.ComposerDir()) {
		t.Error("composer shim not pointing to ~/.pv/composer")
	}
	if !strings.Contains(s, config.ComposerCacheDir()) {
		t.Error("composer shim not pointing to ~/.pv/composer/cache")
	}
	if !strings.Contains(s, config.ComposerPharPath()) {
		t.Error("composer shim not pointing to composer.phar path")
	}
}

func TestWriteShims_ComposerShimSetsIsolation(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	if err := WriteShims(); err != nil {
		t.Fatalf("WriteShims() error = %v", err)
	}

	content, _ := os.ReadFile(filepath.Join(config.BinDir(), "composer"))
	s := string(content)

	// Verify COMPOSER_HOME is set before any exec call.
	homeIdx := strings.Index(s, "export COMPOSER_HOME=")
	execIdx := strings.Index(s, "exec ")
	if homeIdx == -1 || execIdx == -1 {
		t.Fatal("missing COMPOSER_HOME export or exec in shim")
	}
	if homeIdx > execIdx {
		t.Error("COMPOSER_HOME is set after exec — it won't take effect")
	}
}
