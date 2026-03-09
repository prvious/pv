package laravel

import "github.com/prvious/pv/internal/registry"

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
