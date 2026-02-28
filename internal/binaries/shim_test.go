package binaries

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

func TestWriteComposerShim(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	if err := WriteComposerShim(); err != nil {
		t.Fatalf("WriteComposerShim() error = %v", err)
	}

	path := filepath.Join(config.BinDir(), "composer")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("shim not created: %v", err)
	}

	content := string(data)
	if !strings.Contains(content, "composer.phar") {
		t.Error("shim does not contain 'composer.phar'")
	}
	if !strings.Contains(content, filepath.Join(config.BinDir(), "php")) {
		t.Error("shim does not reference php binary")
	}
	if strings.Contains(content, "frankenphp") {
		t.Error("shim should not reference frankenphp anymore")
	}

	info, _ := os.Stat(path)
	if info.Mode().Perm()&0111 == 0 {
		t.Error("shim is not executable")
	}
}

func TestWriteAllShims(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	if err := WriteAllShims(); err != nil {
		t.Fatalf("WriteAllShims() error = %v", err)
	}

	composerPath := filepath.Join(config.BinDir(), "composer")
	if _, err := os.Stat(composerPath); err != nil {
		t.Error("composer shim not created")
	}
}
