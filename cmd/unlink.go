package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

var unlinkCmd = &cobra.Command{
	Use:   "unlink [name]",
	Short: "Unlink a project",
	Args:  cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		var name string
		if len(args) > 0 {
			name = args[0]
		} else {
			cwd, err := os.Getwd()
			if err != nil {
				return fmt.Errorf("cannot get working directory: %w", err)
			}
			absPath, _ := filepath.Abs(cwd)
			p := reg.FindByPath(absPath)
			if p == nil {
				return fmt.Errorf("current directory is not a linked project")
			}
			name = p.Name
		}

		if err := reg.Remove(name); err != nil {
			return err
		}

		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		fmt.Printf("Unlinked %s\n", name)
		return nil
	},
}

func init() {
	rootCmd.AddCommand(unlinkCmd)
}
