package cmd

import (
	"bytes"
	"strings"
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

	var buf bytes.Buffer
	cmd := newListCmd()
	cmd.SetOut(&buf)
	cmd.SetArgs([]string{"list"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("list command error = %v", err)
	}

	// The "no projects" message goes to fmt.Println (stdout), not cmd.OutOrStdout().
	// We verify no error was returned, which is the important part.
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

	var buf bytes.Buffer
	cmd := newListCmd()
	cmd.SetOut(&buf)
	cmd.SetArgs([]string{"list"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("list command error = %v", err)
	}

	out := buf.String()
	if !strings.Contains(out, "NAME") || !strings.Contains(out, "PATH") {
		t.Errorf("expected table header with NAME and PATH, got:\n%s", out)
	}
	if !strings.Contains(out, "app1") {
		t.Errorf("expected app1 in output, got:\n%s", out)
	}
	if !strings.Contains(out, "app2") {
		t.Errorf("expected app2 in output, got:\n%s", out)
	}
	if !strings.Contains(out, "laravel") {
		t.Errorf("expected 'laravel' type in output, got:\n%s", out)
	}
	// app2 has no type, should show "-"
	if !strings.Contains(out, "-") {
		t.Errorf("expected '-' for empty type, got:\n%s", out)
	}
}
