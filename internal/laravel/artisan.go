package laravel

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/projectenv"
)

func HasComposerJSON(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, "composer.json"))
	return err == nil
}

func HasVendorDir(projectPath string) bool {
	info, err := os.Stat(filepath.Join(projectPath, "vendor"))
	return err == nil && info.IsDir()
}

func HasEnvExample(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, ".env.example"))
	return err == nil
}

func HasEnvFile(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, ".env"))
	return err == nil
}

func HasOctaneWorker(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, "public", "frankenphp-worker.php"))
	return err == nil
}

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

func ReadAppKey(projectPath string) string {
	env, err := projectenv.ReadDotEnv(filepath.Join(projectPath, ".env"))
	if err != nil {
		return ""
	}
	return env["APP_KEY"]
}

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

func KeyGenerate(projectPath, phpBin string) error {
	_, err := RunArtisan(projectPath, phpBin, "key:generate", "--force")
	return err
}

func Migrate(projectPath, phpBin string) (string, error) {
	return RunArtisan(projectPath, phpBin, "migrate", "--force")
}

func OctaneInstall(projectPath, phpBin string) error {
	_, err := RunArtisan(projectPath, phpBin, "octane:install", "--server=frankenphp")
	return err
}
