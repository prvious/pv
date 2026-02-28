package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/spf13/cobra"
)

func newInstallCmd() *cobra.Command {
	var force bool
	var tld string

	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	install := &cobra.Command{
		Use:  "install",
		RunE: func(cmd *cobra.Command, args []string) error {
			forceInstall = force
			installTLD = tld
			return installCmd.RunE(cmd, args)
		},
	}
	install.Flags().BoolVar(&force, "force", false, "Reinstall")
	install.Flags().StringVar(&tld, "tld", "test", "TLD")
	root.AddCommand(install)
	return root
}

func TestInstallCmd_Structure(t *testing.T) {
	root := newInstallCmd()
	cmd, _, err := root.Find([]string{"install"})
	if err != nil {
		t.Fatalf("Find() error = %v", err)
	}
	if cmd.Use != "install" {
		t.Errorf("Use = %q, want %q", cmd.Use, "install")
	}
	if cmd.RunE == nil {
		t.Error("RunE is nil")
	}
}

func TestInstallCmd_HasForceFlag(t *testing.T) {
	root := newInstallCmd()
	cmd, _, _ := root.Find([]string{"install"})
	flag := cmd.Flags().Lookup("force")
	if flag == nil {
		t.Error("--force flag not found")
	}
}

func TestInstallCmd_HasTLDFlag(t *testing.T) {
	root := newInstallCmd()
	cmd, _, _ := root.Find([]string{"install"})
	flag := cmd.Flags().Lookup("tld")
	if flag == nil {
		t.Error("--tld flag not found")
	}
}

func TestInstallCmd_AlreadyInstalled(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// Create ~/.pv to simulate existing install.
	if err := os.MkdirAll(filepath.Join(home, ".pv"), 0755); err != nil {
		t.Fatal(err)
	}

	root := newInstallCmd()
	root.SetArgs([]string{"install"})
	err := root.Execute()
	if err == nil {
		t.Fatal("expected error for existing install without --force, got nil")
	}
}
