package laravel

import (
	"path/filepath"

	"github.com/prvious/pv/internal/projectenv"
)

// ResolveDatabaseName reads DB_DATABASE from .env.example.
// Returns sanitized project name if .env.example is missing, has no DB_DATABASE,
// or DB_DATABASE is the generic "laravel" default.
func ResolveDatabaseName(projectPath, projectName string) string {
	envExample := filepath.Join(projectPath, ".env.example")
	env, err := projectenv.ReadDotEnv(envExample)
	if err != nil {
		return projectenv.SanitizeProjectName(projectName)
	}

	dbName, ok := env["DB_DATABASE"]
	if !ok || dbName == "" || dbName == "laravel" {
		return projectenv.SanitizeProjectName(projectName)
	}

	return dbName
}
