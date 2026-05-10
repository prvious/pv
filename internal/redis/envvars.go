package redis

import "strconv"

// EnvVars returns the REDIS_* map injected into a linked project's .env
// when redis is bound. projectName is accepted but unused — kept for
// parallel signature with mysql.EnvVars / postgres.EnvVars so the
// dispatcher in laravel/env.go can treat all three uniformly.
//
// REDIS_PASSWORD is the literal string "null" — Laravel's
// config/database.php reads that as nil, matching the no-auth /
// loopback-only spec posture. Same shape the docker Redis used, so
// projects bound under the old service experience no .env churn on
// migration.
func EnvVars(projectName string) map[string]string {
	_ = projectName // unused — redis uses no project-scoped value
	return map[string]string{
		"REDIS_HOST":     "127.0.0.1",
		"REDIS_PORT":     strconv.Itoa(PortFor()),
		"REDIS_PASSWORD": "null",
	}
}
