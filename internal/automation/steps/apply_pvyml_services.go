package steps

import (
	"fmt"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/redis"
)

// ApplyPvYmlServicesStep binds the services declared in a project's
// pv.yml into the registry. It runs before DetectServicesStep and,
// when active, causes DetectServicesStep to skip via its ShouldRun.
//
// For version-bearing services (postgres, mysql), errors if the
// declared version isn't installed. For single-version services
// (redis, mailpit, rustfs), binds unconditionally — matching the
// existing auto-detect behavior.
type ApplyPvYmlServicesStep struct{}

var _ automation.Step = (*ApplyPvYmlServicesStep)(nil)

func (s *ApplyPvYmlServicesStep) Label() string  { return "Bind services from pv.yml" }
func (s *ApplyPvYmlServicesStep) Gate() string   { return "apply_pvyml_services" }
func (s *ApplyPvYmlServicesStep) Critical() bool { return true }

func (s *ApplyPvYmlServicesStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.ProjectConfig.HasServices()
}

func (s *ApplyPvYmlServicesStep) Run(ctx *automation.Context) (string, error) {
	cfg := ctx.ProjectConfig
	count := 0

	if cfg.Postgresql != nil {
		major := cfg.Postgresql.Version
		if major == "" {
			return "", fmt.Errorf("pv.yml postgresql: version is required")
		}
		if !postgres.IsInstalled(major) {
			return "", fmt.Errorf("pv.yml postgresql %q is not installed — run `pv postgres:install %s`", major, major)
		}
		bindProjectPostgres(ctx.Registry, ctx.ProjectName, major)
		count++
	}

	if cfg.Mysql != nil {
		version := cfg.Mysql.Version
		if version == "" {
			return "", fmt.Errorf("pv.yml mysql: version is required")
		}
		installed, err := mysql.InstalledVersions()
		if err != nil {
			return "", fmt.Errorf("list mysql versions: %w", err)
		}
		found := false
		for _, v := range installed {
			if v == version {
				found = true
				break
			}
		}
		if !found {
			return "", fmt.Errorf("pv.yml mysql %q is not installed — run `pv mysql:install %s`", version, version)
		}
		bindProjectMysql(ctx.Registry, ctx.ProjectName, version)
		count++
	}

	if cfg.Redis != nil {
		if !redis.IsInstalled() {
			return "", fmt.Errorf("pv.yml redis is not installed — run `pv redis:install`")
		}
		bindProjectService(ctx.Registry, ctx.ProjectName, "redis", "redis")
		count++
	}

	if cfg.Mailpit != nil {
		bindProjectService(ctx.Registry, ctx.ProjectName, "mail", "mailpit")
		count++
	}

	if cfg.Rustfs != nil {
		bindProjectService(ctx.Registry, ctx.ProjectName, "s3", "rustfs")
		count++
	}

	return fmt.Sprintf("bound %d service(s) from pv.yml", count), nil
}
