package services

import (
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
)

type Redis struct{}

func (r *Redis) Name() string        { return "redis" }
func (r *Redis) DisplayName() string { return "Redis" }

func (r *Redis) DefaultVersion() string { return "latest" }

func (r *Redis) ImageName(version string) string {
	return "redis:" + version
}

func (r *Redis) ContainerName(version string) string {
	return "pv-redis-" + version
}

func (r *Redis) Port(_ string) int        { return 6379 }
func (r *Redis) ConsolePort(_ string) int  { return 0 }

func (r *Redis) CreateOpts(version string) container.CreateOpts {
	return container.CreateOpts{
		Name:  r.ContainerName(version),
		Image: r.ImageName(version),
		Ports: map[int]int{
			6379: 6379,
		},
		Volumes: map[string]string{
			config.ServiceDataDir("redis", version): "/data",
		},
		Labels: map[string]string{
			"dev.prvious.pv":         "true",
			"dev.prvious.pv.service": "redis",
			"dev.prvious.pv.version": version,
		},
		HealthCmd:      []string{"CMD-SHELL", "redis-cli ping"},
		HealthInterval: "2s",
		HealthTimeout:  "5s",
		HealthRetries:  15,
	}
}

func (r *Redis) EnvVars(_ string, _ int) map[string]string {
	return map[string]string{
		"REDIS_HOST":     "127.0.0.1",
		"REDIS_PORT":     "6379",
		"REDIS_PASSWORD": "null",
	}
}

func (r *Redis) CreateDatabase(_ *container.Engine, _, _ string) error {
	return nil
}

func (r *Redis) HasDatabases() bool { return false }
