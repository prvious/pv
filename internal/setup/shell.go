package setup

import (
	"os"
	"path/filepath"
)

// DetectShell returns the name of the user's shell based on the SHELL env var.
func DetectShell() string {
	shell := os.Getenv("SHELL")
	if shell == "" {
		return "sh"
	}
	return filepath.Base(shell)
}

// ShellConfigFile returns the path to the shell's config file.
func ShellConfigFile(shell string) string {
	home, _ := os.UserHomeDir()
	switch shell {
	case "zsh":
		return filepath.Join(home, ".zshrc")
	case "bash":
		return filepath.Join(home, ".bashrc")
	case "fish":
		return filepath.Join(home, ".config", "fish", "config.fish")
	default:
		return filepath.Join(home, ".profile")
	}
}

// PathExportLine returns the shell-specific line to add ~/.pv/bin and
// ~/.pv/composer/vendor/bin to PATH.
func PathExportLine(shell string) string {
	switch shell {
	case "fish":
		return `set -gx PATH "$HOME/.pv/bin" "$HOME/.pv/composer/vendor/bin" $PATH`
	default:
		return `export PATH="$HOME/.pv/bin:$HOME/.pv/composer/vendor/bin:$PATH"`
	}
}
