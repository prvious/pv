package mysql

import "strconv"

// EnvVars returns the DB_* map injected into a linked project's .env when
// the project is bound to a mysql version. projectName is sanitized by
// the caller (projectenv.SanitizeProjectName).
//
// DB_PASSWORD is empty: mysqld is initialized with --initialize-insecure
// and bound to 127.0.0.1 only, so root has no password. Matches the
// previous Docker MYSQL_ALLOW_EMPTY_PASSWORD posture and the postgres
// trust-auth model.
func EnvVars(projectName, version string) (map[string]string, error) {
	port, err := PortFor(version)
	if err != nil {
		return nil, err
	}
	return map[string]string{
		"DB_CONNECTION": "mysql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       strconv.Itoa(port),
		"DB_DATABASE":   projectName,
		"DB_USERNAME":   "root",
		"DB_PASSWORD":   "",
	}, nil
}
