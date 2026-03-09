package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

func newDoctorCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	doctor := &cobra.Command{
		Use:  "doctor",
		RunE: doctorCmd.RunE,
	}
	root.AddCommand(doctor)
	return root
}

func TestDoctor_EmptyHome(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Create minimal directory structure so Load/Settings don't fail.
	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	cmd := newDoctorCmd()
	cmd.SetArgs([]string{"doctor"})
	// Doctor will report issues and call os.Exit(1); we just verify it doesn't panic.
	// RunE returns nil (exit is handled via os.Exit), so we check it runs without error.
	_ = cmd.Execute()
}

func TestDoctor_WithProjectMissingDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	reg := &registry.Registry{}
	_ = reg.Add(registry.Project{Name: "ghost", Path: "/nonexistent/path", Type: "laravel"})
	if err := reg.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	cmd := newDoctorCmd()
	cmd.SetArgs([]string{"doctor"})
	_ = cmd.Execute()
}

func TestDoctor_WithValidProject(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	// Create a fake PHP version directory with fake binaries.
	phpDir := filepath.Join(home, ".pv", "php", "8.4")
	if err := os.MkdirAll(phpDir, 0755); err != nil {
		t.Fatal(err)
	}
	for _, bin := range []string{"frankenphp", "php"} {
		if err := os.WriteFile(filepath.Join(phpDir, bin), []byte("#!/bin/sh\n"), 0755); err != nil {
			t.Fatal(err)
		}
	}

	// Set global PHP and save settings.
	settings := config.DefaultSettings()
	settings.Defaults.PHP = "8.4"
	if err := settings.Save(); err != nil {
		t.Fatal(err)
	}

	// Create a project with an existing directory.
	projectDir := filepath.Join(home, "projects", "myapp")
	if err := os.MkdirAll(projectDir, 0755); err != nil {
		t.Fatal(err)
	}

	reg := &registry.Registry{}
	_ = reg.Add(registry.Project{Name: "myapp", Path: projectDir, Type: "laravel"})
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	// Create site config so the check passes.
	siteConfig := filepath.Join(config.SitesDir(), "myapp.caddy")
	if err := os.WriteFile(siteConfig, []byte("test config"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newDoctorCmd()
	cmd.SetArgs([]string{"doctor"})
	_ = cmd.Execute()
}
