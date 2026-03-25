package colima

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// Install downloads the Colima binary from GitHub releases.
func Install(client *http.Client, progress func(written, total int64)) error {
	arch := runtime.GOARCH
	platform := runtime.GOOS

	// Map Go arch to Colima release naming.
	// Darwin uses: arm64, x86_64
	// Linux uses: aarch64, x86_64
	colimaArch := arch
	if platform == "linux" && arch == "arm64" {
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

// colimaCmd creates an exec.Command for the Colima binary with Lima on PATH
// and COLIMA_HOME set so all state lives under ~/.pv/.
func colimaCmd(args ...string) *exec.Cmd {
	cmd := exec.Command(config.ColimaPath(), args...)
	cmd.Env = append(os.Environ(),
		"PATH="+config.LimaBinDir()+string(os.PathListSeparator)+os.Getenv("PATH"),
		"COLIMA_HOME="+config.ColimaHomeDir(),
	)
	return cmd
}

// Start starts the Colima VM with the pv profile using the given resource config.
func Start(vm config.VMConfig) error {
	if err := checkVZCompat(); err != nil {
		return err
	}

	vm = vm.WithDefaults()
	cmd := colimaCmd(
		"start", "--profile", "pv",
		"--cpu", strconv.Itoa(vm.CPU),
		"--memory", strconv.Itoa(vm.Memory),
		"--disk", strconv.Itoa(vm.Disk),
		"--vm-type", "vz",
		"--mount-type", "virtiofs",
	)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// Stop stops the Colima VM with the pv profile.
func Stop() error {
	cmd := colimaCmd("stop", "--profile", "pv")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// Delete deletes the Colima VM with the pv profile.
func Delete() error {
	cmd := colimaCmd("delete", "--profile", "pv", "--force")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// IsRunning checks if the Colima VM with the pv profile is running.
func IsRunning() bool {
	cmd := colimaCmd("status", "--profile", "pv")
	return cmd.Run() == nil
}

// EnsureRunning starts Colima if it's not already running. If Start fails
// (e.g. the VM is in a broken state after sleep or macOS update), it attempts
// automatic recovery by force-stopping, deleting, and restarting the VM.
func EnsureRunning(vm config.VMConfig) error {
	if IsRunning() {
		return nil
	}

	err := Start(vm)
	if err == nil {
		return nil
	}

	// VM may be in a broken state. Attempt recovery: force stop, delete, restart.
	fmt.Fprintf(os.Stderr, "Warning: Colima start failed (%v), attempting VM recovery...\n", err)

	_ = forceStop()
	_ = Delete()

	if retryErr := Start(vm); retryErr != nil {
		return fmt.Errorf("cannot start Colima (recovery also failed): %w", retryErr)
	}
	return nil
}

// forceStop force-stops the Colima VM without graceful shutdown.
func forceStop() error {
	cmd := colimaCmd("stop", "--profile", "pv", "--force")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// IsInstalled checks if the Colima binary exists at the expected path.
func IsInstalled() bool {
	_, err := os.Stat(config.ColimaPath())
	return err == nil
}

// Version returns the Colima version string.
func Version() (string, error) {
	out, err := colimaCmd("version").Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}
