package php

import (
	"testing"

	"github.com/spf13/cobra"
)

func TestRegister_AllCommandsPresent(t *testing.T) {
	root := &cobra.Command{Use: "pv"}
	root.AddGroup(&cobra.Group{ID: "php", Title: "PHP"})
	Register(root)

	expected := []string{
		"php:install", "php:download", "php:path",
		"php:update", "php:uninstall", "php:use",
		"php:list", "php:remove", "php:current",
	}
	for _, name := range expected {
		cmd, _, err := root.Find([]string{name})
		if err != nil || cmd.Name() != name {
			t.Errorf("command %q not registered", name)
		}
	}
}
