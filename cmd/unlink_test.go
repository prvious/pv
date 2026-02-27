package cmd

import (
	"testing"

	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

// newUnlinkCmd builds a fresh unlink command for testing.
func newUnlinkCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	unlink := &cobra.Command{
		Use:  "unlink [name]",
		Args: cobra.MaximumNArgs(1),
		RunE: unlinkCmd.RunE,
	}
	root.AddCommand(unlink)
	return root
}

func TestUnlink_ByName(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Seed the registry with a project.
	reg := &registry.Registry{}
	_ = reg.Add(registry.Project{Name: "myapp", Path: "/tmp/myapp"})
	if err := reg.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	cmd := newUnlinkCmd()
	cmd.SetArgs([]string{"unlink", "myapp"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("unlink command error = %v", err)
	}

	loaded, _ := registry.Load()
	if len(loaded.List()) != 0 {
		t.Fatalf("expected 0 projects after unlink, got %d", len(loaded.List()))
	}
}

func TestUnlink_NonExistentName(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cmd := newUnlinkCmd()
	cmd.SetArgs([]string{"unlink", "nope"})
	if err := cmd.Execute(); err == nil {
		t.Fatal("expected error for non-existent name, got nil")
	}
}
