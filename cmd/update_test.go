package cmd

import (
	"testing"

	"github.com/spf13/cobra"
)

func newUpdateCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	update := &cobra.Command{
		Use:  "update",
		RunE: updateCmd.RunE,
	}
	root.AddCommand(update)
	return root
}

func TestUpdateCmd_Structure(t *testing.T) {
	root := newUpdateCmd()
	cmd, _, err := root.Find([]string{"update"})
	if err != nil {
		t.Fatalf("Find() error = %v", err)
	}
	if cmd.Use != "update" {
		t.Errorf("Use = %q, want %q", cmd.Use, "update")
	}
	if cmd.RunE == nil {
		t.Error("RunE is nil")
	}
}
