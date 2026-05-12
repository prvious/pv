package laravel

import (
	"github.com/prvious/pv/internal/projectenv"
)

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
	env, err := projectenv.ReadDotEnv(envPath)
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
	return projectenv.MergeDotEnv(envPath, backupPath, replacements)
}
