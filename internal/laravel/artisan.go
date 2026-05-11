package laravel

import (
	"os"
	"path/filepath"
)

func HasEnvFile(projectPath string) bool {
	_, err := os.Stat(filepath.Join(projectPath, ".env"))
	return err == nil
}
