package php

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/phpenv"
	"github.com/spf13/cobra"
)

var currentCmd = &cobra.Command{
	Use:     "php:current",
	GroupID: "php",
	Short:   "Print the resolved PHP version for the current directory",
	Example: `pv php:current`,
	Args:    cobra.NoArgs,
	RunE: func(cmd *cobra.Command, args []string) error {
		dir, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("cannot determine working directory: %w", err)
		}

		version, err := phpenv.ResolveVersionWalkUp(dir)
		if err != nil {
			return err
		}

		fmt.Fprintln(cmd.OutOrStdout(), version)
		return nil
	},
}
