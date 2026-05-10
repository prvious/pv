package laravel

import (
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

// SmartEnvVars returns Laravel-specific behavioral env vars based on bound services.
// Separate from services.EnvVars() which returns connection details.
func SmartEnvVars(bound *registry.ProjectServices) map[string]string {
	vars := make(map[string]string)
	if bound.Redis {
		vars["CACHE_STORE"] = "redis"
		vars["SESSION_DRIVER"] = "redis"
		vars["QUEUE_CONNECTION"] = "redis"
	}
	if bound.S3 {
		vars["FILESYSTEM_DISK"] = "s3"
	}
	if bound.Mail {
		vars["MAIL_MAILER"] = "smtp"
	}
	return vars
}

// FallbackRule defines a conditional replacement: if a key currently has
// IfValue, it should be replaced with ReplaceWith when the service is removed.
type FallbackRule struct {
	IfValue     string
	ReplaceWith string
}

// FallbackMapping returns rules for safe defaults when a service stops/is removed.
// Only overwrites values pv set — not developer-set values.
func FallbackMapping(serviceName string) map[string]FallbackRule {
	switch serviceName {
	case "redis":
		return map[string]FallbackRule{
			"CACHE_STORE":      {IfValue: "redis", ReplaceWith: "file"},
			"SESSION_DRIVER":   {IfValue: "redis", ReplaceWith: "file"},
			"QUEUE_CONNECTION": {IfValue: "redis", ReplaceWith: "sync"},
		}
	case "s3":
		return map[string]FallbackRule{
			"FILESYSTEM_DISK": {IfValue: "s3", ReplaceWith: "local"},
		}
	case "mail":
		return map[string]FallbackRule{
			"MAIL_MAILER": {IfValue: "smtp", ReplaceWith: "log"},
		}
	default:
		return nil
	}
}

// ApplyFallbacks reads a project's .env, replaces values that match what pv
// would have set for the given service with safe defaults, and writes back.
func ApplyFallbacks(envPath, serviceName string) error {
	rules := FallbackMapping(serviceName)
	if len(rules) == 0 {
		return nil
	}
	env, err := services.ReadDotEnv(envPath)
	if err != nil {
		return err
	}
	replacements := make(map[string]string)
	for key, rule := range rules {
		if currentVal, ok := env[key]; ok && currentVal == rule.IfValue {
			replacements[key] = rule.ReplaceWith
		}
	}
	if len(replacements) == 0 {
		return nil
	}
	backupPath := envPath + ".pv-backup"
	return services.MergeDotEnv(envPath, backupPath, replacements)
}

// UpdateProjectEnvForBinaryService merges connection vars + smart Laravel vars into .env
// for services that run as native binaries (implementing services.BinaryService).
func UpdateProjectEnvForBinaryService(projectPath, projectName, serviceName string, svc services.BinaryService, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	allVars := svc.EnvVars(projectName)
	smartVars := SmartEnvVars(bound)
	for k, v := range smartVars {
		allVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return services.MergeDotEnv(envPath, backupPath, allVars)
}

// UpdateProjectEnvForPostgres mirrors UpdateProjectEnvForBinaryService for
// the postgres native-binary case. postgres has its own EnvVars signature
// (projectName, major) — it doesn't satisfy services.BinaryService.
func UpdateProjectEnvForPostgres(projectPath, projectName, major string, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	pgVars, err := postgres.EnvVars(projectName, major)
	if err != nil {
		return err
	}
	smartVars := SmartEnvVars(bound)
	for k, v := range smartVars {
		pgVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return services.MergeDotEnv(envPath, backupPath, pgVars)
}

// UpdateProjectEnvForMysql mirrors UpdateProjectEnvForPostgres for the mysql
// native-binary case. mysql has its own EnvVars signature (projectName, version)
// — it doesn't satisfy services.BinaryService.
func UpdateProjectEnvForMysql(projectPath, projectName, version string, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	myVars, err := mysql.EnvVars(projectName, version)
	if err != nil {
		return err
	}
	smartVars := SmartEnvVars(bound)
	for k, v := range smartVars {
		myVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return services.MergeDotEnv(envPath, backupPath, myVars)
}

// UpdateProjectEnvForRedis mirrors UpdateProjectEnvForMysql /
// UpdateProjectEnvForPostgres for the redis native-binary case.
// Redis has the simplest signature — EnvVars(projectName) returns a
// constant map (REDIS_HOST/PORT/PASSWORD), no error.
func UpdateProjectEnvForRedis(projectPath, projectName string, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	rVars := redis.EnvVars(projectName)
	smartVars := SmartEnvVars(bound)
	for k, v := range smartVars {
		rVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return services.MergeDotEnv(envPath, backupPath, rVars)
}
