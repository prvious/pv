package mago

import (
	"testing"

	"github.com/spf13/cobra"
)

func TestRegister_AllCommandsPresent(t *testing.T) {
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "mago", Title: "Mago"})
	Register(root)

	expected := []string{"mago:install", "mago:download", "mago:path", "mago:update", "mago:uninstall"}
	for _, name := range expected {
		cmd, _, err := root.Find([]string{name})
		if err != nil || cmd.Name() != name {
			t.Errorf("command %q not registered", name)
		}
	}
}
