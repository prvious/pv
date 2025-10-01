package dnsmasq

import (
	_ "embed"
	"fmt"
	"os"
	"os/exec"
	"runtime"

	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
)

//go:embed dnsmasq.conf.stub
var dnsmasqConfigStub []byte

func init() {
	app.RegisterAction("Setup DNSMasq for .local and .test domains", Setup)
}

func Setup() error {
	log.Info("Setting up dnsmasq for local development domains")

	// Check if dnsmasq is already installed
	if _, err := exec.LookPath("dnsmasq"); err == nil {
		log.Info("dnsmasq is already installed")
	} else {
		log.Info("Installing dnsmasq")
		if err := installDnsmasq(); err != nil {
			return fmt.Errorf("failed to install dnsmasq: %w", err)
		}
		log.Info("dnsmasq installed successfully")
	}

	// Create configuration directory if it doesn't exist
	configDir := "/etc/dnsmasq.d"
	if runtime.GOOS == "darwin" {
		// On macOS, dnsmasq config is typically in /usr/local/etc/dnsmasq.d or /opt/homebrew/etc/dnsmasq.d
		if _, err := os.Stat("/opt/homebrew/etc/dnsmasq.d"); err == nil {
			configDir = "/opt/homebrew/etc/dnsmasq.d"
		} else if _, err := os.Stat("/usr/local/etc/dnsmasq.d"); err == nil {
			configDir = "/usr/local/etc/dnsmasq.d"
		}
	}

	// Create config directory if it doesn't exist
	if err := os.MkdirAll(configDir, 0755); err != nil {
		log.Warn("Failed to create config directory, using current directory", "error", err)
		configDir = "."
	}

	// Write configuration file
	configPath := fmt.Sprintf("%s/dev-domains.conf", configDir)
	if err := os.WriteFile(configPath, dnsmasqConfigStub, 0644); err != nil {
		return fmt.Errorf("failed to write dnsmasq configuration: %w", err)
	}

	log.Info("dnsmasq configuration created", "path", configPath)

	// Provide instructions for completing the setup
	log.Info("To complete the setup:")
	log.Info("1. Start/restart dnsmasq service")

	switch runtime.GOOS {
	case "darwin":
		log.Info("   sudo brew services restart dnsmasq")
		log.Info("2. Configure your system to use dnsmasq:")
		log.Info("   sudo mkdir -p /etc/resolver")
		log.Info("   echo 'nameserver 127.0.0.1' | sudo tee /etc/resolver/local")
		log.Info("   echo 'nameserver 127.0.0.1' | sudo tee /etc/resolver/test")
	case "linux":
		log.Info("   sudo systemctl restart dnsmasq")
		log.Info("2. Configure NetworkManager or systemd-resolved to use dnsmasq")
		log.Info("   Edit /etc/resolv.conf or NetworkManager settings")
	default:
		log.Info("   Restart dnsmasq service using your system's service manager")
		log.Info("2. Configure your system to use 127.0.0.1 as DNS server")
	}

	log.Info("3. Test with: ping myapp.local or ping myapp.test")
	log.Info("Successfully configured dnsmasq for .local and .test domains")

	return nil
}

func installDnsmasq() error {
	var cmd *exec.Cmd

	switch runtime.GOOS {
	case "darwin":
		// Try homebrew
		cmd = exec.Command("brew", "install", "dnsmasq")
	case "linux":
		// Try to detect package manager
		if _, err := exec.LookPath("apt-get"); err == nil {
			cmd = exec.Command("sudo", "apt-get", "install", "-y", "dnsmasq")
		} else if _, err := exec.LookPath("yum"); err == nil {
			cmd = exec.Command("sudo", "yum", "install", "-y", "dnsmasq")
		} else if _, err := exec.LookPath("dnf"); err == nil {
			cmd = exec.Command("sudo", "dnf", "install", "-y", "dnsmasq")
		} else if _, err := exec.LookPath("pacman"); err == nil {
			cmd = exec.Command("sudo", "pacman", "-S", "--noconfirm", "dnsmasq")
		} else {
			return fmt.Errorf("unsupported package manager, please install dnsmasq manually")
		}
	default:
		return fmt.Errorf("unsupported operating system: %s", runtime.GOOS)
	}

	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr

	if err := cmd.Run(); err != nil {
		return fmt.Errorf("installation command failed: %w", err)
	}

	return nil
}
