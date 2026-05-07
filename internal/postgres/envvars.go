package postgres

import "fmt"

// EnvVars returns the DB_* map injected into a linked project's .env.
// projectName is sanitized by the caller (services.SanitizeProjectName).
func EnvVars(projectName, major string) (map[string]string, error) {
	port, err := PortFor(major)
	if err != nil {
		return nil, err
	}
	return map[string]string{
		"DB_CONNECTION": "pgsql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       fmt.Sprintf("%d", port),
		"DB_DATABASE":   projectName,
		"DB_USERNAME":   "postgres",
		"DB_PASSWORD":   "postgres",
	}, nil
}
