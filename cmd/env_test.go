package cmd

import (
	"bytes"
	"path/filepath"
	"strings"
	"testing"

	"github.com/spf13/cobra"
)

func newEnvCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	env := &cobra.Command{
		Use:  "env",
		RunE: envCmd.RunE,
	}
	root.AddCommand(env)
	return root
}

func TestEnv_Zsh(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("SHELL", "/bin/zsh")

	var buf bytes.Buffer
	cmd := newEnvCmd()
	cmd.SetOut(&buf)
	cmd.SetArgs([]string{"env"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("env command error = %v", err)
	}

	out := buf.String()
	if !strings.HasPrefix(out, "export PATH=") {
		t.Errorf("expected 'export PATH=' prefix, got:\n%s", out)
	}
	localBinDir := filepath.Join(home, ".local", "bin")
	if !strings.Contains(out, localBinDir) {
		t.Errorf("expected %q in output, got:\n%s", localBinDir, out)
	}
	binDir := filepath.Join(home, ".pv", "bin")
	if !strings.Contains(out, binDir) {
		t.Errorf("expected %q in output, got:\n%s", binDir, out)
	}
	composerDir := filepath.Join(home, ".pv", "composer", "vendor", "bin")
	if !strings.Contains(out, composerDir) {
		t.Errorf("expected %q in output, got:\n%s", composerDir, out)
	}
	if !strings.Contains(out, "$PATH") {
		t.Errorf("expected $PATH in output, got:\n%s", out)
	}
}

func TestEnv_Bash(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("SHELL", "/bin/bash")

	var buf bytes.Buffer
	cmd := newEnvCmd()
	cmd.SetOut(&buf)
	cmd.SetArgs([]string{"env"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("env command error = %v", err)
	}

	out := buf.String()
	if !strings.HasPrefix(out, "export PATH=") {
		t.Errorf("expected 'export PATH=' prefix, got:\n%s", out)
	}
}

func TestEnv_Fish(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("SHELL", "/usr/local/bin/fish")

	var buf bytes.Buffer
	cmd := newEnvCmd()
	cmd.SetOut(&buf)
	cmd.SetArgs([]string{"env"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("env command error = %v", err)
	}

	out := buf.String()
	if !strings.HasPrefix(out, "fish_add_path") {
		t.Errorf("expected 'fish_add_path' prefix, got:\n%s", out)
	}
	localBinDir := filepath.Join(home, ".local", "bin")
	if !strings.Contains(out, localBinDir) {
		t.Errorf("expected %q in output, got:\n%s", localBinDir, out)
	}
	binDir := filepath.Join(home, ".pv", "bin")
	if !strings.Contains(out, binDir) {
		t.Errorf("expected %q in output, got:\n%s", binDir, out)
	}
}

func TestEnv_NoShellEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("SHELL", "")

	var buf bytes.Buffer
	cmd := newEnvCmd()
	cmd.SetOut(&buf)
	cmd.SetArgs([]string{"env"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("env command error = %v", err)
	}

	out := buf.String()
	// Falls back to sh-compatible export
	if !strings.HasPrefix(out, "export PATH=") {
		t.Errorf("expected 'export PATH=' prefix for fallback, got:\n%s", out)
	}
}
