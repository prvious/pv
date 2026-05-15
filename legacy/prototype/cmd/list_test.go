package cmd

import (
	"testing"

	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

// newListCmd builds a fresh list command for testing.
func newListCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	list := &cobra.Command{
		Use:  "list",
		RunE: listCmd.RunE,
	}
	root.AddCommand(list)
	return root
}

func TestList_NoProjects(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	cmd := newListCmd()
	cmd.SetArgs([]string{"list"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("list command error = %v", err)
	}
}

func TestList_WithProjects(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	reg := &registry.Registry{}
	_ = reg.Add(registry.Project{Name: "app1", Path: "/srv/app1", Type: "laravel"})
	_ = reg.Add(registry.Project{Name: "app2", Path: "/srv/app2"})
	if err := reg.Save(); err != nil {
		t.Fatalf("Save() error = %v", err)
	}

	cmd := newListCmd()
	cmd.SetArgs([]string{"list"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("list command error = %v", err)
	}
}
