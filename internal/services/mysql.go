package services

import (
	"fmt"
	"strconv"
	"strings"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
)

type MySQL struct{}

func (m *MySQL) Name() string        { return "mysql" }
func (m *MySQL) DisplayName() string { return "MySQL" }

func (m *MySQL) DefaultVersion() string { return "latest" }

func (m *MySQL) ImageName(version string) string {
	return "mysql:" + version
}

func (m *MySQL) ContainerName(version string) string {
	return "pv-mysql-" + version
}

// Port returns the host port for a MySQL version.
// Scheme: 33000 + patch version. For "latest", returns 33000.
func (m *MySQL) Port(version string) int {
	if version == "latest" {
		return 33000
	}
	parts := strings.Split(version, ".")
	if len(parts) >= 3 {
		if patch, err := strconv.Atoi(parts[2]); err == nil {
			return 33000 + patch
		}
	}
	// Fallback for versions like "8.0" or "8"
	return 33000
}

func (m *MySQL) ConsolePort(_ string) int  { return 0 }
func (m *MySQL) WebRoutes() []WebRoute     { return nil }

func (m *MySQL) CreateOpts(version string) container.CreateOpts {
	port := m.Port(version)
	return container.CreateOpts{
		Name:  m.ContainerName(version),
		Image: m.ImageName(version),
		Env: []string{
			"MYSQL_ALLOW_EMPTY_PASSWORD=yes",
		},
		Ports: map[int]int{
			port: 3306,
		},
		Volumes: map[string]string{
			config.ServiceDataDir("mysql", version): "/var/lib/mysql",
		},
		Labels: map[string]string{
			"dev.prvious.pv":         "true",
			"dev.prvious.pv.service": "mysql",
			"dev.prvious.pv.version": version,
		},
		HealthCmd:      []string{"CMD-SHELL", "mysqladmin ping -h 127.0.0.1"},
		HealthInterval: "2s",
		HealthTimeout:  "5s",
		HealthRetries:  15,
	}
}

func (m *MySQL) EnvVars(projectName string, port int) map[string]string {
	return map[string]string{
		"DB_CONNECTION": "mysql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       fmt.Sprintf("%d", port),
		"DB_DATABASE":   projectName,
		"DB_USERNAME":   "root",
		"DB_PASSWORD":   "",
	}
}

func (m *MySQL) CreateDatabase(engine *container.Engine, containerID, dbName string) error {
	// Implemented via Docker exec in the container engine layer.
	// The command: mysql -u root -e "CREATE DATABASE IF NOT EXISTS <dbName>"
	_ = engine
	_ = containerID
	_ = dbName
	return nil
}

func (m *MySQL) HasDatabases() bool { return true }
