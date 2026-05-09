package services

import (
	"fmt"
	"regexp"
	"sort"
	"strings"

	"github.com/prvious/pv/internal/container"
)

var safeIdentifier = regexp.MustCompile(`[^a-zA-Z0-9_]`)

// WebRoute maps a subdomain under pv.{tld} to a local port.
// For example, {Subdomain: "s3", Port: 9001} routes s3.pv.test → 127.0.0.1:9001.
type WebRoute struct {
	Subdomain string
	Port      int
}

type Service interface {
	Name() string
	DisplayName() string
	ImageName(version string) string
	ContainerName(version string) string
	DefaultVersion() string
	Port(version string) int
	ConsolePort(version string) int
	WebRoutes() []WebRoute // HTTP endpoints exposed under *.pv.{tld}
	CreateOpts(version string) container.CreateOpts
	EnvVars(projectName string, port int) map[string]string
	CreateDatabase(engine *container.Engine, containerID, dbName string) error
	HasDatabases() bool
}

var registry = map[string]Service{
	"redis": &Redis{},
}

func Lookup(name string) (Service, error) {
	svc, ok := registry[name]
	if !ok {
		return nil, fmt.Errorf("unknown service %q (available: %s)", name, strings.Join(Available(), ", "))
	}
	return svc, nil
}

// Available returns the union of Docker and binary service names, sorted.
// A set deduplicates entries in case a name ever appears in both registries —
// not currently the case, but not prevented by the type system either.
func Available() []string {
	seen := make(map[string]struct{}, len(registry)+len(binaryRegistry))
	for n := range registry {
		seen[n] = struct{}{}
	}
	for n := range binaryRegistry {
		seen[n] = struct{}{}
	}
	names := make([]string, 0, len(seen))
	for n := range seen {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
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
