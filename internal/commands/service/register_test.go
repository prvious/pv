package service

import (
	"testing"

	"github.com/spf13/cobra"
)

func TestRegister_AllCommandsPresent(t *testing.T) {
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "service", Title: "Services"})
	Register(root)

	expected := []string{
		"service:add", "service:start", "service:stop",
		"service:status", "service:list", "service:env",
		"service:remove", "service:destroy", "service:logs",
	}
	for _, name := range expected {
		cmd, _, err := root.Find([]string{name})
		if err != nil || cmd.Name() != name {
			t.Errorf("command %q not registered", name)
		}
	}
}
