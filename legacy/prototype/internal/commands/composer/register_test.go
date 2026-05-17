package composer

import (
	"testing"

	"github.com/spf13/cobra"
)

func TestRegister_AllCommandsPresent(t *testing.T) {
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "composer", Title: "Composer"})
	Register(root)

	expected := []string{"composer:install", "composer:download", "composer:path", "composer:update", "composer:uninstall"}
	for _, name := range expected {
		cmd, _, err := root.Find([]string{name})
		if err != nil || cmd.Name() != name {
			t.Errorf("command %q not registered", name)
		}
	}
}
