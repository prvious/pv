package laravel

import (
	"os"
	"path/filepath"
	"testing"
)

func TestHasComposerJSON(t *testing.T) {
	dir := t.TempDir()
	if HasComposerJSON(dir) {
		t.Error("expected false for empty dir")
	}
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte("{}"), 0644)
	if !HasComposerJSON(dir) {
		t.Error("expected true when composer.json exists")
	}
}

func TestHasVendorDir(t *testing.T) {
	dir := t.TempDir()
	if HasVendorDir(dir) {
		t.Error("expected false for empty dir")
	}
	os.MkdirAll(filepath.Join(dir, "vendor"), 0755)
	if !HasVendorDir(dir) {
		t.Error("expected true when vendor/ exists")
	}
}

func TestHasEnvExample(t *testing.T) {
	dir := t.TempDir()
	if HasEnvExample(dir) {
		t.Error("expected false for empty dir")
	}
	os.WriteFile(filepath.Join(dir, ".env.example"), []byte("APP_KEY=\n"), 0644)
	if !HasEnvExample(dir) {
		t.Error("expected true when .env.example exists")
	}
}

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

func TestHasOctaneWorker(t *testing.T) {
	dir := t.TempDir()
	if HasOctaneWorker(dir) {
		t.Error("expected false for empty dir")
	}
	os.MkdirAll(filepath.Join(dir, "public"), 0755)
	os.WriteFile(filepath.Join(dir, "public", "frankenphp-worker.php"), []byte("<?php"), 0644)
	if !HasOctaneWorker(dir) {
		t.Error("expected true when worker exists")
	}
}

func TestHasOctanePackage(t *testing.T) {
	dir := t.TempDir()
	if HasOctanePackage(dir) {
		t.Error("expected false for empty dir")
	}

	// Without octane
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte(`{"require":{"laravel/framework":"^11.0"}}`), 0644)
	if HasOctanePackage(dir) {
		t.Error("expected false without octane in require")
	}

	// With octane
	os.WriteFile(filepath.Join(dir, "composer.json"), []byte(`{"require":{"laravel/framework":"^11.0","laravel/octane":"^2.0"}}`), 0644)
	if !HasOctanePackage(dir) {
		t.Error("expected true with octane in require")
	}
}

func TestReadAppKey_Empty(t *testing.T) {
	dir := t.TempDir()
	// No .env at all
	if key := ReadAppKey(dir); key != "" {
		t.Errorf("expected empty key, got %q", key)
	}

	// .env with empty APP_KEY
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=\n"), 0644)
	if key := ReadAppKey(dir); key != "" {
		t.Errorf("expected empty key, got %q", key)
	}
}

func TestReadAppKey_Set(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_KEY=base64:abc123def456\n"), 0644)

	key := ReadAppKey(dir)
	if key != "base64:abc123def456" {
		t.Errorf("ReadAppKey() = %q, want %q", key, "base64:abc123def456")
	}
}
