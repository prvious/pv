package detection

import (
	"encoding/json"
	"os"
	"path/filepath"
)

type composerJSON struct {
	Require map[string]string `json:"require"`
}

// Detect examines the project at projectPath and returns a type string.
// Detection is best-effort: missing or unreadable files are silently skipped.
func Detect(projectPath string) string {
	composerPath := filepath.Join(projectPath, "composer.json")
	data, err := os.ReadFile(composerPath)
	if err == nil {
		var c composerJSON
		if json.Unmarshal(data, &c) == nil {
			if _, ok := c.Require["laravel/framework"]; ok {
				if _, hasOctane := c.Require["laravel/octane"]; hasOctane {
					workerPath := filepath.Join(projectPath, "public", "frankenphp-worker.php")
					if _, err := os.Stat(workerPath); err == nil {
						return "laravel-octane"
					}
				}
				return "laravel"
			}
			return "php"
		}
	}

	indexPath := filepath.Join(projectPath, "index.html")
	if _, err := os.Stat(indexPath); err == nil {
		return "static"
	}

	return ""
}
