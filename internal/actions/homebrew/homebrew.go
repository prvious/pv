package homebrew

import (
	"fmt"
	"os"
	"os/exec"
	"runtime"

	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
)

func init() {
	app.RegisterAction("Install Homebrew", Setup)
}

func Setup() error {
	log.Info("Installing Homebrew package manager")

	// Check if Homebrew is already installed
	if _, err := exec.LookPath("brew"); err == nil {
		log.Info("Homebrew is already installed")
		return nil
	}

	// Check operating system
	if runtime.GOOS == "windows" {
		return fmt.Errorf("Homebrew is not supported on Windows")
	}

	log.Info("Running Homebrew installation script...")

	// Run the official Homebrew installation script
	installScript := "curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh | /bin/bash"
	cmd := exec.Command("/bin/bash", "-c", installScript)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr

	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to install Homebrew: %w", err)
	}

	log.Info("Homebrew installed successfully")
	return nil
}
