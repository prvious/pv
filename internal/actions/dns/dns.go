package dns

import (
	"fmt"
	"os"
	"os/exec"
	"runtime"

	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
)

func init() {
	app.RegisterAction("dns:install", Setup)
}

func Setup() error {
	log.Info("Installing CoreDNS")

	// Detect OS and architecture
	goos := runtime.GOOS
	goarch := runtime.GOARCH

	log.Info("Detected system", "os", goos, "arch", goarch)

	// Check if CoreDNS is already installed
	if _, err := exec.LookPath("coredns"); err == nil {
		log.Info("CoreDNS is already installed")
		return nil
	}

	// Install CoreDNS based on OS
	switch goos {
	case "linux":
		return installLinux()
	case "darwin":
		return installMacOS()
	case "windows":
		return installWindows()
	default:
		return fmt.Errorf("unsupported operating system: %s", goos)
	}
}

func installLinux() error {
	log.Info("Installing CoreDNS on Linux")

	// Check if running on a system with package manager
	if _, err := exec.LookPath("apt-get"); err == nil {
		log.Info("Using apt-get to install CoreDNS")
		cmd := exec.Command("sudo", "apt-get", "update")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("failed to update apt: %w", err)
		}

		cmd = exec.Command("sudo", "apt-get", "install", "-y", "coredns")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("failed to install CoreDNS via apt: %w", err)
		}

		log.Info("CoreDNS installed successfully via apt-get")
		return nil
	}

	if _, err := exec.LookPath("yum"); err == nil {
		log.Info("Using yum to install CoreDNS")
		cmd := exec.Command("sudo", "yum", "install", "-y", "coredns")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("failed to install CoreDNS via yum: %w", err)
		}

		log.Info("CoreDNS installed successfully via yum")
		return nil
	}

	return fmt.Errorf("no supported package manager found (apt-get or yum)")
}

func installMacOS() error {
	log.Info("Installing CoreDNS on macOS")

	// Check if Homebrew is installed
	if _, err := exec.LookPath("brew"); err != nil {
		return fmt.Errorf("Homebrew is not installed. Please install Homebrew first: https://brew.sh")
	}

	log.Info("Using Homebrew to install CoreDNS")
	cmd := exec.Command("brew", "install", "coredns")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to install CoreDNS via Homebrew: %w", err)
	}

	log.Info("CoreDNS installed successfully via Homebrew")
	return nil
}

func installWindows() error {
	log.Info("Installing CoreDNS on Windows")

	// Check if Chocolatey is installed
	if _, err := exec.LookPath("choco"); err != nil {
		return fmt.Errorf("Chocolatey is not installed. Please install Chocolatey first: https://chocolatey.org")
	}

	log.Info("Using Chocolatey to install CoreDNS")
	cmd := exec.Command("choco", "install", "coredns", "-y")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to install CoreDNS via Chocolatey: %w", err)
	}

	log.Info("CoreDNS installed successfully via Chocolatey")
	return nil
}
