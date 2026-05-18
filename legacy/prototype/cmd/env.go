package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

var envCmd = &cobra.Command{
	Use:     "env",
	GroupID: "core",
	Short:   "Print shell configuration for pv",
	Long:    "Print shell commands to configure PATH for pv.",
	Example: `# Add to your .zshrc or .bashrc
eval "$(pv env)"`,
	RunE: func(cmd *cobra.Command, args []string) error {
		shell := detectShell()

		binDir := config.BinDir()
		composerBinDir := config.ComposerBinDir()
		composerHome := config.ComposerDir()
		composerCacheDir := config.ComposerCacheDir()

		switch shell {
		case "fish":
			fmt.Fprintf(cmd.OutOrStdout(), "fish_add_path -g %q %q;\n", binDir, composerBinDir)
			fmt.Fprintf(cmd.OutOrStdout(), "set -gx COMPOSER_HOME %q;\n", composerHome)
			fmt.Fprintf(cmd.OutOrStdout(), "set -gx COMPOSER_CACHE_DIR %q;\n", composerCacheDir)
		default:
			fmt.Fprintf(cmd.OutOrStdout(), "export PATH=%q:%q:\"$PATH\";\n", binDir, composerBinDir)
			fmt.Fprintf(cmd.OutOrStdout(), "export COMPOSER_HOME=%q;\n", composerHome)
			fmt.Fprintf(cmd.OutOrStdout(), "export COMPOSER_CACHE_DIR=%q;\n", composerCacheDir)
		}

		return nil
	},
}

// detectShell returns the name of the user's login shell.
func detectShell() string {
	shell := os.Getenv("SHELL")
	if shell == "" {
		return "sh"
	}
	return filepath.Base(shell)
}

func init() {
	rootCmd.AddCommand(envCmd)
}
