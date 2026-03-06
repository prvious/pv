package services

import (
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
)

type RustFS struct{}

func (r *RustFS) Name() string        { return "rustfs" }
func (r *RustFS) DisplayName() string { return "RustFS" }

func (r *RustFS) DefaultVersion() string { return "latest" }

func (r *RustFS) ImageName(version string) string {
	return "rustfs/rustfs:" + version
}

func (r *RustFS) ContainerName(version string) string {
	return "pv-rustfs-" + version
}

func (r *RustFS) Port(_ string) int        { return 9000 }
func (r *RustFS) ConsolePort(_ string) int  { return 9001 }

func (r *RustFS) CreateOpts(version string) container.CreateOpts {
	return container.CreateOpts{
		Name:  r.ContainerName(version),
		Image: r.ImageName(version),
		Env: []string{
			"RUSTFS_ROOT_USER=minioadmin",
			"RUSTFS_ROOT_PASSWORD=minioadmin",
		},
		Ports: map[int]int{
			9000: 9000,
			9001: 9001,
		},
		Volumes: map[string]string{
			config.ServiceDataDir("rustfs", version): "/data",
		},
		Labels: map[string]string{
			"dev.prvious.pv":         "true",
			"dev.prvious.pv.service": "rustfs",
			"dev.prvious.pv.version": version,
		},
		Cmd:            []string{"server", "/data", "--console-address", ":9001"},
		HealthCmd:      []string{"CMD-SHELL", "curl -f http://localhost:9000/minio/health/live"},
		HealthInterval: "2s",
		HealthTimeout:  "5s",
		HealthRetries:  15,
	}
}

func (r *RustFS) EnvVars(projectName string, _ int) map[string]string {
	return map[string]string{
		"AWS_ACCESS_KEY_ID":             "minioadmin",
		"AWS_SECRET_ACCESS_KEY":         "minioadmin",
		"AWS_DEFAULT_REGION":            "us-east-1",
		"AWS_BUCKET":                    projectName,
		"AWS_ENDPOINT":                  "http://127.0.0.1:9000",
		"AWS_USE_PATH_STYLE_ENDPOINT":   "true",
	}
}

func (r *RustFS) CreateDatabase(_ *container.Engine, _, _ string) error {
	return nil
}

func (r *RustFS) HasDatabases() bool { return false }
