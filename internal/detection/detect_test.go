package detection

import (
	"os"
	"path/filepath"
	"testing"
)

// scaffold creates a temp dir with the given files (path → content) and returns the dir path.
func scaffold(t *testing.T, files map[string]string) string {
	t.Helper()
	dir := t.TempDir()
	for name, content := range files {
		full := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(full), 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(full, []byte(content), 0644); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func TestDetect_LaravelOctane(t *testing.T) {
	dir := scaffold(t, map[string]string{
		"composer.json": `{"require":{"laravel/framework":"^11.0","laravel/octane":"^2.0"}}`,
		"public/frankenphp-worker.php": "<?php // worker",
	})
	if got := Detect(dir); got != "laravel-octane" {
		t.Errorf("Detect() = %q, want %q", got, "laravel-octane")
	}
}

func TestDetect_LaravelOctaneWithoutWorker(t *testing.T) {
	dir := scaffold(t, map[string]string{
		"composer.json": `{"require":{"laravel/framework":"^11.0","laravel/octane":"^2.0"}}`,
	})
	if got := Detect(dir); got != "laravel" {
		t.Errorf("Detect() = %q, want %q", got, "laravel")
	}
}

func TestDetect_Laravel(t *testing.T) {
	dir := scaffold(t, map[string]string{
		"composer.json": `{"require":{"laravel/framework":"^11.0"}}`,
	})
	if got := Detect(dir); got != "laravel" {
		t.Errorf("Detect() = %q, want %q", got, "laravel")
	}
}

func TestDetect_GenericPHP(t *testing.T) {
	dir := scaffold(t, map[string]string{
		"composer.json": `{"require":{"some/package":"^1.0"}}`,
	})
	if got := Detect(dir); got != "php" {
		t.Errorf("Detect() = %q, want %q", got, "php")
	}
}

func TestDetect_Static(t *testing.T) {
	dir := scaffold(t, map[string]string{
		"index.html": "<html></html>",
	})
	if got := Detect(dir); got != "static" {
		t.Errorf("Detect() = %q, want %q", got, "static")
	}
}

func TestDetect_EmptyDir(t *testing.T) {
	dir := t.TempDir()
	if got := Detect(dir); got != "" {
		t.Errorf("Detect() = %q, want %q", got, "")
	}
}

func TestDetect_ComposerWinsOverIndexHTML(t *testing.T) {
	dir := scaffold(t, map[string]string{
		"composer.json": `{"require":{"some/package":"^1.0"}}`,
		"index.html":    "<html></html>",
	})
	if got := Detect(dir); got != "php" {
		t.Errorf("Detect() = %q, want %q", got, "php")
	}
}
