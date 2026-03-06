package colima

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"runtime"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// Install downloads the Colima binary from GitHub releases.
func Install(client *http.Client, progress func(written, total int64)) error {
	arch := runtime.GOARCH
	platform := runtime.GOOS

	// Map Go arch to Colima naming.
	colimaArch := arch
	if arch == "arm64" {
		colimaArch = "aarch64"
	} else if arch == "amd64" {
		colimaArch = "x86_64"
	}

	// Get latest version tag.
	// Capitalize platform name: "darwin" -> "Darwin".
	platformName := strings.ToUpper(platform[:1]) + platform[1:]
	url := fmt.Sprintf("https://github.com/abiosoft/colima/releases/latest/download/colima-%s-%s", platformName, colimaArch)

	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return err
	}

	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("cannot download Colima: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("cannot download Colima: HTTP %d", resp.StatusCode)
	}

	colimaPath := config.ColimaPath()
	f, err := os.OpenFile(colimaPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0755)
	if err != nil {
		return err
	}
	defer f.Close()

	if progress != nil {
		pr := &progressReader{r: resp.Body, total: resp.ContentLength, progress: progress}
		_, err = io.Copy(f, pr)
	} else {
		_, err = io.Copy(f, resp.Body)
	}
	return err
}

type progressReader struct {
	r        io.Reader
	total    int64
	written  int64
	progress func(written, total int64)
}

func (pr *progressReader) Read(p []byte) (int, error) {
	n, err := pr.r.Read(p)
	pr.written += int64(n)
	if pr.progress != nil {
		pr.progress(pr.written, pr.total)
	}
	return n, err
}

// Start starts the Colima VM with the pv profile.
func Start() error {
	cmd := exec.Command(config.ColimaPath(),
		"start", "--profile", "pv",
		"--cpu", "2",
		"--memory", "2",
		"--disk", "60",
		"--vm-type", "vz",
		"--mount-type", "virtiofs",
	)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// Stop stops the Colima VM with the pv profile.
func Stop() error {
	cmd := exec.Command(config.ColimaPath(), "stop", "--profile", "pv")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// Delete deletes the Colima VM with the pv profile.
func Delete() error {
	cmd := exec.Command(config.ColimaPath(), "delete", "--profile", "pv", "--force")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// IsRunning checks if the Colima VM with the pv profile is running.
func IsRunning() bool {
	cmd := exec.Command(config.ColimaPath(), "status", "--profile", "pv")
	return cmd.Run() == nil
}

// EnsureRunning starts Colima if it's not already running.
func EnsureRunning() error {
	if IsRunning() {
		return nil
	}
	return Start()
}

// IsInstalled checks if the Colima binary exists at the expected path.
func IsInstalled() bool {
	_, err := os.Stat(config.ColimaPath())
	return err == nil
}

// Version returns the Colima version string.
func Version() (string, error) {
	out, err := exec.Command(config.ColimaPath(), "version").Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}
