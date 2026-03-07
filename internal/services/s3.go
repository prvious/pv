package services

import (
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
)

type S3 struct{}

func (s *S3) Name() string        { return "s3" }
func (s *S3) DisplayName() string { return "S3 Storage" }

func (s *S3) DefaultVersion() string { return "latest" }

func (s *S3) ImageName(version string) string {
	return "rustfs/rustfs:" + version
}

func (s *S3) ContainerName(version string) string {
	return "pv-s3-" + version
}

func (s *S3) Port(_ string) int        { return 9000 }
func (s *S3) ConsolePort(_ string) int { return 9001 }

func (s *S3) WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
}

func (s *S3) CreateOpts(version string) container.CreateOpts {
	return container.CreateOpts{
		Name:  s.ContainerName(version),
		Image: s.ImageName(version),
		Env: []string{
			"RUSTFS_ROOT_USER=minioadmin",
			"RUSTFS_ROOT_PASSWORD=minioadmin",
		},
		Ports: map[int]int{
			9000: 9000,
			9001: 9001,
		},
		Volumes: map[string]string{
			config.ServiceDataDir("s3", version): "/data",
		},
		Labels: map[string]string{
			"dev.prvious.pv":         "true",
			"dev.prvious.pv.service": "s3",
			"dev.prvious.pv.version": version,
		},
		Cmd:            []string{"server", "/data", "--console-address", ":9001"},
		HealthCmd:      []string{"CMD-SHELL", "curl -f http://localhost:9000/minio/health/live"},
		HealthInterval: "2s",
		HealthTimeout:  "5s",
		HealthRetries:  15,
	}
}

func (s *S3) EnvVars(projectName string, _ int) map[string]string {
	return map[string]string{
		"AWS_ACCESS_KEY_ID":           "minioadmin",
		"AWS_SECRET_ACCESS_KEY":       "minioadmin",
		"AWS_DEFAULT_REGION":          "us-east-1",
		"AWS_BUCKET":                  projectName,
		"AWS_ENDPOINT":                "http://127.0.0.1:9000",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
	}
}

func (s *S3) CreateDatabase(_ *container.Engine, _, _ string) error {
	return nil
}

func (s *S3) HasDatabases() bool { return false }
