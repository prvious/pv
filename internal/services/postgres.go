package services

import (
	"context"
	"fmt"
	"strconv"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
)

type Postgres struct{}

func (p *Postgres) Name() string        { return "postgres" }
func (p *Postgres) DisplayName() string { return "PostgreSQL" }

func (p *Postgres) DefaultVersion() string { return "18-alpine" }

func (p *Postgres) ImageName(version string) string {
	return "postgres:" + version
}

func (p *Postgres) ContainerName(version string) string {
	return "pv-postgres-" + version
}

// Port returns the host port for a PostgreSQL version.
// Scheme: 54000 + major version. For "latest", returns 54000.
func (p *Postgres) Port(version string) int {
	if version == "latest" {
		return 54000
	}
	major, err := strconv.Atoi(version)
	if err == nil {
		return 54000 + major
	}
	return 54000
}

func (p *Postgres) ConsolePort(_ string) int { return 0 }
func (p *Postgres) WebRoutes() []WebRoute    { return nil }

func (p *Postgres) CreateOpts(version string) container.CreateOpts {
	port := p.Port(version)
	return container.CreateOpts{
		Name:  p.ContainerName(version),
		Image: p.ImageName(version),
		Env: []string{
			"POSTGRES_USER=postgres",
			"POSTGRES_PASSWORD=postgres",
			"POSTGRES_HOST_AUTH_METHOD=trust",
		},
		Ports: map[int]int{
			port: 5432,
		},
		Volumes: map[string]string{
			config.ServiceDataDir("postgres", version): "/var/lib/postgresql",
		},
		Labels: map[string]string{
			"dev.prvious.pv":         "true",
			"dev.prvious.pv.service": "postgres",
			"dev.prvious.pv.version": version,
		},
		HealthCmd:      []string{"CMD-SHELL", "pg_isready -d postgres -U postgres"},
		HealthInterval: "3s",
		HealthTimeout:  "5s",
		HealthRetries:  20,
	}
}

func (p *Postgres) EnvVars(projectName string, port int) map[string]string {
	return map[string]string{
		"DB_CONNECTION": "pgsql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       fmt.Sprintf("%d", port),
		"DB_DATABASE":   projectName,
		"DB_USERNAME":   "postgres",
		"DB_PASSWORD":   "",
	}
}

func (p *Postgres) CreateDatabase(engine *container.Engine, containerName, dbName string) error {
	return engine.Exec(context.Background(), containerName, []string{
		"psql", "-U", "postgres", "-c",
		fmt.Sprintf("CREATE DATABASE \"%s\"", dbName),
	})
}

func (p *Postgres) HasDatabases() bool { return true }
