package laravel

import (
	"fmt"
	"os/exec"
	"strings"
)

// ComposerInstall runs composer install in the project directory.
func ComposerInstall(projectPath string) (string, error) {
	cmd := exec.Command("composer", "install", "--no-interaction", "--prefer-dist")
	cmd.Dir = projectPath
	out, err := cmd.CombinedOutput()
	trimmed := strings.TrimSpace(string(out))
	if err != nil {
		return trimmed, fmt.Errorf("%w: %s", err, trimmed)
	}
	return trimmed, nil
}
