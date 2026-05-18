package daemon

import (
	"testing"

	"github.com/spf13/cobra"
)

func TestRegister_AllCommandsPresent(t *testing.T) {
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "daemon", Title: "Daemon"})
	Register(root)

	expected := []string{"daemon:enable", "daemon:disable", "daemon:restart"}
	for _, name := range expected {
		cmd, _, err := root.Find([]string{name})
		if err != nil || cmd.Name() != name {
			t.Errorf("command %q not registered", name)
		}
	}
}
