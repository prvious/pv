package laravel

import (
	"os"
	"path/filepath"
	"testing"
)

func TestHasEnvFile(t *testing.T) {
	dir := t.TempDir()
	if HasEnvFile(dir) {
		t.Error("expected false for empty dir")
	}
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=\n"), 0644)
	if !HasEnvFile(dir) {
		t.Error("expected true when .env exists")
	}
}
