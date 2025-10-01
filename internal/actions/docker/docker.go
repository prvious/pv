package docker

import (
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"strings"

	"github.com/charmbracelet/log"
	"github.com/prvious/pv/internal/app"
)

func init() {
	app.RegisterAction("Install/Update Docker", Setup)
}

func Setup() error {
	log.Info("Starting Docker installation/update process")

	// Detect OS and architecture
	osType := runtime.GOOS
	arch := runtime.GOARCH

	log.Info("System detected", "os", osType, "arch", arch)

	// Check if Docker is already installed
	dockerInstalled := checkDockerInstalled()

	if dockerInstalled {
		log.Info("Docker is already installed, checking version...")
		if err := showDockerVersion(); err != nil {
			log.Warn("Could not get Docker version", "error", err)
		}
		log.Info("Attempting to update Docker...")
		return updateDocker(osType)
	}

	log.Info("Docker not found, proceeding with installation...")
	return installDocker(osType, arch)
}

func checkDockerInstalled() bool {
	cmd := exec.Command("docker", "--version")
	err := cmd.Run()
	return err == nil
}

func showDockerVersion() error {
	cmd := exec.Command("docker", "--version")
	output, err := cmd.Output()
	if err != nil {
		return err
	}
	log.Info("Current Docker version", "version", strings.TrimSpace(string(output)))
	return nil
}

func updateDocker(osType string) error {
	switch osType {
	case "darwin":
		return updateDockerMacOS()
	case "linux":
		return updateDockerLinux()
	default:
		return fmt.Errorf("unsupported operating system: %s", osType)
	}
}

func installDocker(osType, arch string) error {
	switch osType {
	case "darwin":
		return installDockerMacOS(arch)
	case "linux":
		return installDockerLinux(arch)
	default:
		return fmt.Errorf("unsupported operating system: %s", osType)
	}
}

func updateDockerMacOS() error {
	log.Info("Updating Docker on macOS")

	// Check if Homebrew is available
	if checkCommandExists("brew") {
		log.Info("Using Homebrew to update Docker...")
		cmd := exec.Command("brew", "upgrade", "docker", "--cask")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			log.Warn("Homebrew upgrade failed, Docker might already be up to date", "error", err)
		} else {
			log.Info("Docker updated successfully via Homebrew")
		}
		return nil
	}

	log.Info("Please update Docker Desktop manually from the Docker menu or download the latest version from https://www.docker.com/products/docker-desktop")
	return nil
}

func installDockerMacOS(arch string) error {
	log.Info("Installing Docker on macOS", "arch", arch)

	// Check if Homebrew is available
	if !checkCommandExists("brew") {
		return fmt.Errorf("Homebrew is required for Docker installation on macOS. Please install Homebrew first: https://brew.sh")
	}

	log.Info("Installing Docker Desktop via Homebrew...")
	cmd := exec.Command("brew", "install", "--cask", "docker")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr

	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to install Docker via Homebrew: %w", err)
	}

	log.Info("Docker Desktop installed successfully!")
	log.Info("Please start Docker Desktop from your Applications folder")
	return nil
}

func updateDockerLinux() error {
	log.Info("Updating Docker on Linux")

	distro := detectLinuxDistro()
	log.Info("Linux distribution detected", "distro", distro)

	switch distro {
	case "ubuntu", "debian":
		return updateDockerDebian()
	case "fedora", "centos", "rhel":
		return updateDockerRHEL()
	default:
		log.Warn("Unknown Linux distribution, attempting generic update")
		return updateDockerDebian()
	}
}

func installDockerLinux(arch string) error {
	log.Info("Installing Docker on Linux", "arch", arch)

	distro := detectLinuxDistro()
	log.Info("Linux distribution detected", "distro", distro)

	switch distro {
	case "ubuntu", "debian":
		return installDockerDebian(arch)
	case "fedora", "centos", "rhel":
		return installDockerRHEL(arch)
	default:
		log.Warn("Unknown Linux distribution, attempting Debian-based installation")
		return installDockerDebian(arch)
	}
}

func detectLinuxDistro() string {
	// Try to read /etc/os-release
	data, err := os.ReadFile("/etc/os-release")
	if err != nil {
		return "unknown"
	}

	content := string(data)
	lines := strings.Split(content, "\n")

	for _, line := range lines {
		if strings.HasPrefix(line, "ID=") {
			distro := strings.TrimPrefix(line, "ID=")
			distro = strings.Trim(distro, "\"")
			return strings.ToLower(distro)
		}
	}

	return "unknown"
}

func updateDockerDebian() error {
	log.Info("Updating Docker packages on Debian-based system")

	// Update package list
	log.Info("Updating package list...")
	cmd := exec.Command("sudo", "apt-get", "update")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to update package list: %w", err)
	}

	// Upgrade Docker packages
	log.Info("Upgrading Docker packages...")
	cmd = exec.Command("sudo", "apt-get", "install", "-y", "--only-upgrade", "docker-ce", "docker-ce-cli", "containerd.io", "docker-buildx-plugin", "docker-compose-plugin")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		log.Warn("Docker upgrade completed with warnings", "error", err)
	} else {
		log.Info("Docker updated successfully")
	}

	return nil
}

func installDockerDebian(arch string) error {
	log.Info("Installing Docker on Debian-based system")

	// Update package list
	log.Info("Updating package list...")
	cmd := exec.Command("sudo", "apt-get", "update")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to update package list: %w", err)
	}

	// Install prerequisites
	log.Info("Installing prerequisites...")
	cmd = exec.Command("sudo", "apt-get", "install", "-y",
		"ca-certificates", "curl", "gnupg", "lsb-release")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to install prerequisites: %w", err)
	}

	// Add Docker's GPG key
	log.Info("Adding Docker's official GPG key...")
	cmd = exec.Command("sudo", "mkdir", "-p", "/etc/apt/keyrings")
	if err := cmd.Run(); err != nil {
		log.Warn("Failed to create keyrings directory", "error", err)
	}

	keyCmd := `curl -fsSL https://download.docker.com/linux/$(lsb_release -is | tr '[:upper:]' '[:lower:]')/gpg | sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg`
	cmd = exec.Command("bash", "-c", keyCmd)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to add Docker GPG key: %w", err)
	}

	// Set up the repository
	log.Info("Setting up Docker repository...")
	repoCmd := `echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/$(lsb_release -is | tr '[:upper:]' '[:lower:]') $(lsb_release -cs) stable" | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null`
	cmd = exec.Command("bash", "-c", repoCmd)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to set up Docker repository: %w", err)
	}

	// Update package list again
	log.Info("Updating package list with Docker repository...")
	cmd = exec.Command("sudo", "apt-get", "update")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to update package list: %w", err)
	}

	// Install Docker
	log.Info("Installing Docker Engine, CLI, containerd, and plugins...")
	cmd = exec.Command("sudo", "apt-get", "install", "-y",
		"docker-ce", "docker-ce-cli", "containerd.io",
		"docker-buildx-plugin", "docker-compose-plugin")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("failed to install Docker: %w", err)
	}

	// Add current user to docker group
	log.Info("Adding current user to docker group...")
	username := os.Getenv("USER")
	if username == "" {
		username = os.Getenv("LOGNAME")
	}
	if username != "" {
		cmd = exec.Command("sudo", "usermod", "-aG", "docker", username)
		if err := cmd.Run(); err != nil {
			log.Warn("Failed to add user to docker group", "error", err)
		} else {
			log.Info("User added to docker group", "user", username)
			log.Info("Please log out and back in for group changes to take effect")
		}
	}

	log.Info("Docker installed successfully!")
	return nil
}

func updateDockerRHEL() error {
	log.Info("Updating Docker packages on RHEL-based system")

	// Update Docker packages
	log.Info("Upgrading Docker packages...")
	cmd := exec.Command("sudo", "dnf", "update", "-y", "docker-ce", "docker-ce-cli", "containerd.io", "docker-buildx-plugin", "docker-compose-plugin")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		// Try with yum if dnf is not available
		cmd = exec.Command("sudo", "yum", "update", "-y", "docker-ce", "docker-ce-cli", "containerd.io", "docker-buildx-plugin", "docker-compose-plugin")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			log.Warn("Docker upgrade completed with warnings", "error", err)
		}
	}

	log.Info("Docker updated successfully")
	return nil
}

func installDockerRHEL(arch string) error {
	log.Info("Installing Docker on RHEL-based system")

	// Install prerequisites
	log.Info("Installing prerequisites...")
	cmd := exec.Command("sudo", "dnf", "install", "-y", "dnf-plugins-core")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		// Try with yum if dnf is not available
		cmd = exec.Command("sudo", "yum", "install", "-y", "yum-utils")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("failed to install prerequisites: %w", err)
		}
	}

	// Add Docker repository
	log.Info("Adding Docker repository...")
	cmd = exec.Command("sudo", "dnf", "config-manager", "--add-repo", "https://download.docker.com/linux/fedora/docker-ce.repo")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		// Try with yum-config-manager
		cmd = exec.Command("sudo", "yum-config-manager", "--add-repo", "https://download.docker.com/linux/centos/docker-ce.repo")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("failed to add Docker repository: %w", err)
		}
	}

	// Install Docker
	log.Info("Installing Docker Engine, CLI, containerd, and plugins...")
	cmd = exec.Command("sudo", "dnf", "install", "-y",
		"docker-ce", "docker-ce-cli", "containerd.io",
		"docker-buildx-plugin", "docker-compose-plugin")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		// Try with yum
		cmd = exec.Command("sudo", "yum", "install", "-y",
			"docker-ce", "docker-ce-cli", "containerd.io",
			"docker-buildx-plugin", "docker-compose-plugin")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("failed to install Docker: %w", err)
		}
	}

	// Start and enable Docker
	log.Info("Starting Docker service...")
	cmd = exec.Command("sudo", "systemctl", "start", "docker")
	if err := cmd.Run(); err != nil {
		log.Warn("Failed to start Docker service", "error", err)
	}

	cmd = exec.Command("sudo", "systemctl", "enable", "docker")
	if err := cmd.Run(); err != nil {
		log.Warn("Failed to enable Docker service", "error", err)
	}

	// Add current user to docker group
	log.Info("Adding current user to docker group...")
	username := os.Getenv("USER")
	if username == "" {
		username = os.Getenv("LOGNAME")
	}
	if username != "" {
		cmd = exec.Command("sudo", "usermod", "-aG", "docker", username)
		if err := cmd.Run(); err != nil {
			log.Warn("Failed to add user to docker group", "error", err)
		} else {
			log.Info("User added to docker group", "user", username)
			log.Info("Please log out and back in for group changes to take effect")
		}
	}

	log.Info("Docker installed successfully!")
	return nil
}

func checkCommandExists(cmd string) bool {
	_, err := exec.LookPath(cmd)
	return err == nil
}
