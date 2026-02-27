package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

// newLinkCmd builds a fresh link command not tied to the package-level rootCmd.
func newLinkCmd() *cobra.Command {
	var name string

	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	link := &cobra.Command{
		Use:  "link [path]",
		Args: cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			// Sync local flag → package-level var before delegating.
			linkName = name
			return linkCmd.RunE(cmd, args)
		},
	}
	link.Flags().StringVar(&name, "name", "", "Custom name for the project")
	root.AddCommand(link)
	return root
}

func TestLink_ExplicitPathAndName(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := t.TempDir()

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if len(reg.List()) != 1 {
		t.Fatalf("expected 1 project, got %d", len(reg.List()))
	}

	absPath, _ := filepath.Abs(projDir)
	p := reg.Find("myapp")
	if p == nil {
		t.Fatal("project 'myapp' not found in registry")
	}
	if p.Path != absPath {
		t.Errorf("path = %q, want %q", p.Path, absPath)
	}
}

func TestLink_NonExistentPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", "/does/not/exist"})
	if err := cmd.Execute(); err == nil {
		t.Fatal("expected error for non-existent path, got nil")
	}
}

func TestLink_FileNotDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	f := filepath.Join(t.TempDir(), "file.txt")
	if err := os.WriteFile(f, []byte("hi"), 0644); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", f})
	if err := cmd.Execute(); err == nil {
		t.Fatal("expected error for file path, got nil")
	}
}

func TestLink_DuplicateName(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := t.TempDir()

	cmd1 := newLinkCmd()
	cmd1.SetArgs([]string{"link", projDir, "--name", "dup"})
	if err := cmd1.Execute(); err != nil {
		t.Fatalf("first link error = %v", err)
	}

	projDir2 := t.TempDir()
	cmd2 := newLinkCmd()
	cmd2.SetArgs([]string{"link", projDir2, "--name", "dup"})
	if err := cmd2.Execute(); err == nil {
		t.Fatal("expected error for duplicate name, got nil")
	}
}

func TestLink_DefaultsToBasename(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	projDir := filepath.Join(t.TempDir(), "cool-project")
	if err := os.MkdirAll(projDir, 0755); err != nil {
		t.Fatal(err)
	}

	cmd := newLinkCmd()
	cmd.SetArgs([]string{"link", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("link command error = %v", err)
	}

	reg, _ := registry.Load()
	p := reg.Find("cool-project")
	if p == nil {
		t.Fatal("expected project named 'cool-project'")
	}
}
