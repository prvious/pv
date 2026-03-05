package setup

import (
	"fmt"
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

// PrintPathInstructions prints the instructions for adding pv to the user's PATH.
func PrintPathInstructions() {
	shell := DetectShell()
	configFile := ShellConfigFile(shell)

	fmt.Println("Add pv to your PATH:")
	fmt.Println()
	switch shell {
	case "fish":
		fmt.Printf("  echo 'pv env | source' >> %s\n", configFile)
	default:
		fmt.Printf("  echo 'eval \"$(pv env)\"' >> %s\n", configFile)
	}
	fmt.Printf("  source %s\n", configFile)
	fmt.Println()
	fmt.Println("Or configure your current session:")
	fmt.Println()
	switch shell {
	case "fish":
		fmt.Println("  pv env | source")
	default:
		fmt.Println("  eval \"$(pv env)\"")
	}
}
