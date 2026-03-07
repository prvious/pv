package php

import (
	"fmt"

	"github.com/prvious/pv/internal/tools"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var pathRemove bool

var pathCmd = &cobra.Command{
	Use:     "php:path",
	GroupID: "php",
	Short: "Expose or remove PHP and FrankenPHP from PATH",
	RunE: func(cmd *cobra.Command, args []string) error {
		php := tools.MustGet("php")
		fp := tools.MustGet("frankenphp")

		if pathRemove {
			if err := tools.Unexpose(php); err != nil {
				return err
			}
			if err := tools.Unexpose(fp); err != nil {
				return err
			}
			ui.Success("PHP and FrankenPHP removed from PATH")
			return nil
		}

		if err := tools.Expose(php); err != nil {
			return fmt.Errorf("cannot expose PHP: %w", err)
		}
		if err := tools.Expose(fp); err != nil {
			return fmt.Errorf("cannot expose FrankenPHP: %w", err)
		}
		ui.Success("PHP and FrankenPHP added to PATH")
		return nil
	},
}

func init() {
	pathCmd.Flags().BoolVar(&pathRemove, "remove", false, "Remove from PATH instead of adding")
}
