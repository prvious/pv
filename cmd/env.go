package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/spf13/cobra"
)

var envCmd = &cobra.Command{
	Use:   "env",
	Short: "Print shell configuration for pv",
	Long: `Print shell commands to configure PATH for pv.

Add this to your shell config (.zshrc, .bashrc, config.fish):

  eval "$(pv env)"

Or run it directly to configure your current session.`,
	RunE: func(cmd *cobra.Command, args []string) error {
		shell := detectShell()
		home, err := os.UserHomeDir()
		if err != nil {
			return fmt.Errorf("cannot determine home directory: %w", err)
		}

		localBinDir := filepath.Join(home, ".local", "bin")
		binDir := filepath.Join(home, ".pv", "bin")
		composerBinDir := filepath.Join(home, ".pv", "composer", "vendor", "bin")

		switch shell {
		case "fish":
			fmt.Fprintf(cmd.OutOrStdout(), "fish_add_path -g %q %q %q;\n", localBinDir, binDir, composerBinDir)
		default:
			fmt.Fprintf(cmd.OutOrStdout(), "export PATH=%q:%q:%q:\"$PATH\";\n", localBinDir, binDir, composerBinDir)
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
