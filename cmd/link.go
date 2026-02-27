package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/registry"
	"github.com/spf13/cobra"
)

var linkName string

var linkCmd = &cobra.Command{
	Use:   "link [path]",
	Short: "Link a project directory",
	Args:  cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		path := "."
		if len(args) > 0 {
			path = args[0]
		}

		absPath, err := filepath.Abs(path)
		if err != nil {
			return fmt.Errorf("cannot resolve path: %w", err)
		}

		info, err := os.Stat(absPath)
		if err != nil {
			return fmt.Errorf("path does not exist: %w", err)
		}
		if !info.IsDir() {
			return fmt.Errorf("%s is not a directory", absPath)
		}

		name := linkName
		if name == "" {
			name = filepath.Base(absPath)
		}

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		if err := reg.Add(registry.Project{
			Name: name,
			Path: absPath,
		}); err != nil {
			return err
		}

		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		fmt.Printf("Linked %s → %s\n", name, absPath)
		return nil
	},
}

func init() {
	linkCmd.Flags().StringVar(&linkName, "name", "", "Custom name for the project")
	rootCmd.AddCommand(linkCmd)
}
