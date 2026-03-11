package steps

import (
	"fmt"
	"path/filepath"
	"slices"
	"strings"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

// DetectServicesStep reads the project .env file, detects backing services,
// and binds them to the project in the registry.
type DetectServicesStep struct{}

var _ automation.Step = (*DetectServicesStep)(nil)

func (s *DetectServicesStep) Label() string  { return "Detect and bind services" }
func (s *DetectServicesStep) Gate() string   { return "detect_services" }
func (s *DetectServicesStep) Critical() bool { return false }

func (s *DetectServicesStep) ShouldRun(_ *automation.Context) bool {
	return true
}

func (s *DetectServicesStep) Run(ctx *automation.Context) (string, error) {
	envPath := filepath.Join(ctx.ProjectPath, ".env")
	envVars, err := services.ReadDotEnv(envPath)
	if err != nil {
		return "no .env found", nil
	}

	var bound int
	dbName := services.SanitizeProjectName(ctx.ProjectName)

	// Detect MySQL.
	if conn, ok := envVars["DB_CONNECTION"]; ok && conn == "mysql" {
		if svcKey := findServiceByName(ctx.Registry, "mysql"); svcKey != "" {
			bindProjectService(ctx.Registry, ctx.ProjectName, "mysql", svcKey)
			bound++
		}
	}

	// Detect PostgreSQL.
	if conn, ok := envVars["DB_CONNECTION"]; ok && conn == "pgsql" {
		if svcKey := findServiceByName(ctx.Registry, "postgres"); svcKey != "" {
			bindProjectService(ctx.Registry, ctx.ProjectName, "postgres", svcKey)
			bound++
		}
	}

	// Detect Redis.
	if _, ok := envVars["REDIS_HOST"]; ok {
		if svcKey := findServiceByName(ctx.Registry, "redis"); svcKey != "" {
			bindProjectService(ctx.Registry, ctx.ProjectName, "redis", svcKey)
			bound++
		}
	}

	// Detect Mail.
	if host, ok := envVars["MAIL_HOST"]; ok && (strings.Contains(host, "localhost") || strings.Contains(host, "127.0.0.1")) {
		if svcKey := findServiceByName(ctx.Registry, "mail"); svcKey != "" {
			bindProjectService(ctx.Registry, ctx.ProjectName, "mail", svcKey)
			bound++
		}
	}

	// Detect S3.
	if endpoint, ok := envVars["AWS_ENDPOINT"]; ok && (strings.Contains(endpoint, "localhost") || strings.Contains(endpoint, "127.0.0.1")) {
		if svcKey := findServiceByName(ctx.Registry, "s3"); svcKey != "" {
			bindProjectService(ctx.Registry, ctx.ProjectName, "s3", svcKey)
			bound++
		}
	}

	// Auto-create database entry.
	for i := range ctx.Registry.Projects {
		if ctx.Registry.Projects[i].Name == ctx.ProjectName && ctx.Registry.Projects[i].Services != nil {
			if ctx.Registry.Projects[i].Services.MySQL != "" || ctx.Registry.Projects[i].Services.Postgres != "" {
				if !slices.Contains(ctx.Registry.Projects[i].Databases, dbName) {
					ctx.Registry.Projects[i].Databases = append(ctx.Registry.Projects[i].Databases, dbName)
				}
			}
			break
		}
	}

	if bound == 0 {
		return "no services detected", nil
	}
	return fmt.Sprintf("bound %d services", bound), nil
}

func findServiceByName(reg *registry.Registry, name string) string {
	for key := range reg.Services {
		keyName := key
		if idx := strings.Index(key, ":"); idx > 0 {
			keyName = key[:idx]
		}
		if keyName == name {
			return key
		}
	}
	return ""
}

func bindProjectService(reg *registry.Registry, projectName, svcType, svcKey string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		version := "latest"
		if idx := strings.Index(svcKey, ":"); idx > 0 {
			version = svcKey[idx+1:]
		}
		switch svcType {
		case "mysql":
			reg.Projects[i].Services.MySQL = version
		case "postgres":
			reg.Projects[i].Services.Postgres = version
		case "redis":
			reg.Projects[i].Services.Redis = true
		case "mail":
			reg.Projects[i].Services.Mail = true
		case "s3":
			reg.Projects[i].Services.S3 = true
		}
		break
	}
}
