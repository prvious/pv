package registry

import "strings"

func ServiceKey(name, version string) string {
	if version == "" || version == "latest" {
		return name
	}
	return name + ":" + version
}

// ParseServiceKey splits a registry key into service name and version.
// For "mysql:8.4" returns ("mysql", "8.4"). For "redis" returns ("redis", "latest").
func ParseServiceKey(key string) (name, version string) {
	if idx := strings.Index(key, ":"); idx > 0 {
		return key[:idx], key[idx+1:]
	}
	return key, "latest"
}
