package colima

import (
	"testing"

	"github.com/spf13/cobra"
)

func TestRegister_AllCommandsPresent(t *testing.T) {
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "colima", Title: "Colima"})
	Register(root)

	expected := []string{"colima:install", "colima:download", "colima:path", "colima:update", "colima:uninstall"}
	for _, name := range expected {
		cmd, _, err := root.Find([]string{name})
		if err != nil || cmd.Name() != name {
			t.Errorf("command %q not registered", name)
		}
	}
}
