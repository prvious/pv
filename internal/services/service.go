package services

import (
	"fmt"

	"github.com/prvious/pv/internal/container"
)

type Service interface {
	Name() string
	DisplayName() string
	ImageName(version string) string
	ContainerName(version string) string
	DefaultVersion() string
	Port(version string) int
	ConsolePort(version string) int
	CreateOpts(version string) container.CreateOpts
	EnvVars(projectName string, port int) map[string]string
	CreateDatabase(engine *container.Engine, containerID, dbName string) error
	HasDatabases() bool
}

var registry = map[string]Service{
	"mysql":    &MySQL{},
	"postgres": &Postgres{},
	"redis":    &Redis{},
	"rustfs":   &RustFS{},
}

func Lookup(name string) (Service, error) {
	svc, ok := registry[name]
	if !ok {
		return nil, fmt.Errorf("unknown service %q (available: mysql, postgres, redis, rustfs)", name)
	}
	return svc, nil
}

func Available() []string {
	return []string{"mysql", "postgres", "redis", "rustfs"}
}

// ServiceKey returns the registry key for a service instance.
// For versioned services: "mysql:8.0.32". For unversioned: "redis".
func ServiceKey(name, version string) string {
	if version == "" || version == "latest" {
		return name
	}
	return name + ":" + version
}
