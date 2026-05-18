package php

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/spf13/cobra"
)

func scaffold(t *testing.T) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}
	s := config.DefaultSettings()
	if err := s.Save(); err != nil {
		t.Fatalf("Save settings error = %v", err)
	}
}

func installFakeVersion(t *testing.T, version string) {
	t.Helper()
	dir := config.PhpVersionDir(version)
	if err := os.MkdirAll(dir, 0755); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"frankenphp", "php"} {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, []byte("#!/bin/sh\nexit 0\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}
}

func runCurrent(t *testing.T) (string, error) {
	t.Helper()
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "php", Title: "PHP"})
	Register(root)

	var stdout bytes.Buffer
	root.SetOut(&stdout)

	// Override the current command to write to our buffer.
	cmd, _, err := root.Find([]string{"php:current"})
	if err != nil {
		t.Fatalf("cannot find php:current: %v", err)
	}
	cmd.SetOut(&stdout)

	root.SetArgs([]string{"php:current"})
	if err := root.Execute(); err != nil {
		return "", err
	}
	return strings.TrimSpace(stdout.String()), nil
}

func TestCurrent_PvYml(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := phpenv.SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	// Change to project directory.
	orig, _ := os.Getwd()
	if err := os.Chdir(projDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	got, err := runCurrent(t)
	if err != nil {
		t.Fatalf("php:current error = %v", err)
	}
	if got != "8.3" {
		t.Errorf("php:current = %q, want %q", got, "8.3")
	}
}

func TestCurrent_ComposerJSON(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := phpenv.SetGlobal("8.3"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	composer := `{"require": {"php": "^8.3"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0644); err != nil {
		t.Fatal(err)
	}

	orig, _ := os.Getwd()
	if err := os.Chdir(projDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	got, err := runCurrent(t)
	if err != nil {
		t.Fatalf("php:current error = %v", err)
	}
	if got != "8.4" {
		t.Errorf("php:current = %q, want %q (highest matching ^8.3)", got, "8.4")
	}
}

func TestCurrent_GlobalFallback(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")
	if err := phpenv.SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	emptyDir := t.TempDir()

	orig, _ := os.Getwd()
	if err := os.Chdir(emptyDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	got, err := runCurrent(t)
	if err != nil {
		t.Fatalf("php:current error = %v", err)
	}
	if got != "8.4" {
		t.Errorf("php:current = %q, want %q", got, "8.4")
	}
}

func TestCurrent_WalksUpToPvYml(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := phpenv.SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	subDir := filepath.Join(projDir, "src", "Models")
	if err := os.MkdirAll(subDir, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}

	orig, _ := os.Getwd()
	if err := os.Chdir(subDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	got, err := runCurrent(t)
	if err != nil {
		t.Fatalf("php:current error = %v", err)
	}
	if got != "8.3" {
		t.Errorf("php:current = %q, want %q (should walk up to pv.yml)", got, "8.3")
	}
}

func TestCurrent_WalksUpToComposer(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := phpenv.SetGlobal("8.3"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	subDir := filepath.Join(projDir, "app", "Http", "Controllers")
	if err := os.MkdirAll(subDir, 0755); err != nil {
		t.Fatal(err)
	}
	composer := `{"require": {"php": "^8.4"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0644); err != nil {
		t.Fatal(err)
	}

	orig, _ := os.Getwd()
	if err := os.Chdir(subDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	got, err := runCurrent(t)
	if err != nil {
		t.Fatalf("php:current error = %v", err)
	}
	if got != "8.4" {
		t.Errorf("php:current = %q, want %q", got, "8.4")
	}
}

func TestCurrent_PvYmlPriorityOverComposer(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.3")
	installFakeVersion(t, "8.4")
	if err := phpenv.SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, "pv.yml"), []byte("php: \"8.3\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	composer := `{"require": {"php": "^8.4"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0644); err != nil {
		t.Fatal(err)
	}

	orig, _ := os.Getwd()
	if err := os.Chdir(projDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	got, err := runCurrent(t)
	if err != nil {
		t.Fatalf("php:current error = %v", err)
	}
	if got != "8.3" {
		t.Errorf("php:current = %q, want %q (pv.yml should beat composer.json)", got, "8.3")
	}
}

func TestCurrent_NoGlobalSet(t *testing.T) {
	scaffold(t)
	// No PHP installed, no global set, no pv.yml.

	emptyDir := t.TempDir()

	orig, _ := os.Getwd()
	if err := os.Chdir(emptyDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	_, err := runCurrent(t)
	if err == nil {
		t.Error("expected error when no PHP version is configured")
	}
}

func TestCurrent_RejectsArgs(t *testing.T) {
	scaffold(t)

	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "php", Title: "PHP"})
	Register(root)
	root.SetArgs([]string{"php:current", "extra-arg"})

	err := root.Execute()
	if err == nil {
		t.Error("expected error when passing arguments to php:current")
	}
}

func TestCurrent_OutputToStdout(t *testing.T) {
	scaffold(t)
	installFakeVersion(t, "8.4")
	if err := phpenv.SetGlobal("8.4"); err != nil {
		t.Fatal(err)
	}

	emptyDir := t.TempDir()
	orig, _ := os.Getwd()
	if err := os.Chdir(emptyDir); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chdir(orig) })

	got, err := runCurrent(t)
	if err != nil {
		t.Fatalf("php:current error = %v", err)
	}
	// Should be a clean version string, no extra output.
	if got != "8.4" {
		t.Errorf("expected clean version output %q, got %q", "8.4", got)
	}
	// Should not contain newlines (besides the trailing one trimmed by runCurrent).
	if strings.Contains(got, "\n") {
		t.Errorf("output should be a single line, got %q", got)
	}
}
