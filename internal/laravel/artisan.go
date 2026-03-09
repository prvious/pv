package laravel

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/services"
)

// HasComposerJSON checks whether composer.json exists in the project.
func HasComposerJSON(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, "composer.json"))
	return err == nil
}

// HasVendorDir checks whether the vendor/ directory exists in the project.
func HasVendorDir(projectPath string) bool {
	info, err := os.Stat(filepath.Join(projectPath, "vendor"))
	return err == nil && info.IsDir()
}

// HasEnvExample checks whether .env.example exists in the project.
func HasEnvExample(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, ".env.example"))
	return err == nil
}

// HasEnvFile checks whether .env exists in the project.
func HasEnvFile(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, ".env"))
	return err == nil
}

// HasOctaneWorker checks whether public/frankenphp-worker.php exists.
func HasOctaneWorker(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, "public", "frankenphp-worker.php"))
	return err == nil
}

// HasOctanePackage checks whether laravel/octane is in composer.json require.
func HasOctanePackage(projectPath string) bool {
	data, err := os.ReadFile(filepath.Join(projectPath, "composer.json"))
	if err != nil {
		return false
	}
	var c struct {
		Require map[string]string `json:"require"`
	}
	if json.Unmarshal(data, &c) != nil {
		return false
	}
	_, ok := c.Require["laravel/octane"]
	return ok
}

// ReadAppKey reads APP_KEY from the project's .env file.
// Returns empty string if .env is missing or APP_KEY is not set.
func ReadAppKey(projectPath string) string {
	env, err := services.ReadDotEnv(filepath.Join(projectPath, ".env"))
	if err != nil {
		return ""
	}
	return env["APP_KEY"]
}

// RunArtisan executes an artisan command in the project directory.
func RunArtisan(projectPath, phpBin string, args ...string) (string, error) {
	cmdArgs := append([]string{filepath.Join(projectPath, "artisan")}, args...)
	cmd := exec.Command(phpBin, cmdArgs...)
	cmd.Dir = projectPath
	out, err := cmd.CombinedOutput()
	trimmed := strings.TrimSpace(string(out))
	if err != nil {
		return trimmed, fmt.Errorf("%w: %s", err, trimmed)
	}
	return trimmed, nil
}

// KeyGenerate runs artisan key:generate.
func KeyGenerate(projectPath, phpBin string) error {
	_, err := RunArtisan(projectPath, phpBin, "key:generate", "--force")
	return err
}

// Migrate runs artisan migrate --force.
func Migrate(projectPath, phpBin string) (string, error) {
	return RunArtisan(projectPath, phpBin, "migrate", "--force")
}

// OctaneInstall runs artisan octane:install --server=frankenphp.
func OctaneInstall(projectPath, phpBin string) error {
	_, err := RunArtisan(projectPath, phpBin, "octane:install", "--server=frankenphp")
	return err
}
