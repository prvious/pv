package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
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

		if err := caddy.RemoveSiteConfig(name); err != nil {
			return fmt.Errorf("cannot remove site config: %w", err)
		}
		if err := caddy.GenerateCaddyfile(); err != nil {
			return fmt.Errorf("cannot generate Caddyfile: %w", err)
		}

		fmt.Printf("Unlinked %s\n", name)

		if server.IsRunning() {
			if err := server.ReconfigureServer(); err != nil {
				fmt.Fprintf(os.Stderr, "Warning: could not reconfigure server: %v\n", err)
			}
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(unlinkCmd)
}
