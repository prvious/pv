package services

import (
	"fmt"
	"regexp"
	"sort"
	"strings"
)

var safeIdentifier = regexp.MustCompile(`[^a-zA-Z0-9_]`)

// WebRoute maps a subdomain under pv.{tld} to a local port.
// For example, {Subdomain: "s3", Port: 9001} routes s3.pv.test → 127.0.0.1:9001.
type WebRoute struct {
	Subdomain string
	Port      int
}

// Available returns the names of all registered services, sorted.
// All services now run as native binaries supervised by the daemon.
func Available() []string {
	names := make([]string, 0, len(binaryRegistry))
	for n := range binaryRegistry {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}

// Lookup returns the BinaryService registered under name, or an error
// listing the available services.
func Lookup(name string) (BinaryService, error) {
	if svc, ok := binaryRegistry[name]; ok {
		return svc, nil
	}
	return nil, fmt.Errorf("unknown service %q (available: %s)", name, strings.Join(Available(), ", "))
}

// SanitizeProjectName converts a directory name to a database-safe identifier.
// Only alphanumeric characters and underscores are kept; everything else is stripped.
func SanitizeProjectName(name string) string {
	name = strings.ReplaceAll(name, "-", "_")
	return safeIdentifier.ReplaceAllString(name, "")
}

// ServiceKey returns the registry key for a service instance.
// For versioned services: "mysql:8.0.32". For unversioned: "redis".
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
