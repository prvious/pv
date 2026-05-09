package steps

import (
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
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
		if os.IsNotExist(err) {
			return "no .env found", nil
		}
		ui.Subtle(fmt.Sprintf("Could not read %s: %v", envPath, err))
		return "skipped (.env unreadable)", nil
	}

	var bound int
	dbName := services.SanitizeProjectName(ctx.ProjectName)

	// Postgres takes a separate path: it's a native binary, not a docker service.
	if envVars["DB_CONNECTION"] == "pgsql" {
		majors, err := postgres.InstalledMajors()
		if err == nil && len(majors) > 0 {
			// Prefer the highest installed major.
			major := majors[len(majors)-1]
			bindProjectPostgres(ctx.Registry, ctx.ProjectName, major)
			bound++
		} else {
			ui.Subtle("postgres detected but not installed. Run: pv postgres:install")
		}
	}

	type probe struct {
		match  bool
		name   string
		addCmd string
	}

	probes := []probe{
		{envVars["DB_CONNECTION"] == "mysql", "mysql", "pv service:add mysql"},
		{envVars["REDIS_HOST"] != "", "redis", "pv service:add redis"},
		{
			func() bool {
				h := envVars["MAIL_HOST"]
				return h != "" && (strings.Contains(h, "localhost") || strings.Contains(h, "127.0.0.1"))
			}(),
			"mail", "pv mailpit:install",
		},
		{
			func() bool {
				e := envVars["AWS_ENDPOINT"]
				return e != "" && (strings.Contains(e, "localhost") || strings.Contains(e, "127.0.0.1"))
			}(),
			"s3", "pv rustfs:install",
		},
	}

	for _, p := range probes {
		if !p.match {
			continue
		}
		if svcKey := findServiceByName(ctx.Registry, p.name); svcKey != "" {
			bindProjectService(ctx.Registry, ctx.ProjectName, p.name, svcKey)
			bound++
		} else {
			ui.Subtle(fmt.Sprintf("%s detected but no service running. Run: %s", p.name, p.addCmd))
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

func bindProjectPostgres(reg *registry.Registry, projectName, major string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		reg.Projects[i].Services.Postgres = major
		return
	}
}
