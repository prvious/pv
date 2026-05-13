package steps

import (
	"fmt"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/rustfs"
)

// ApplyPvYmlServicesStep binds the services declared in a project's
// pv.yml into the registry.
//
// For version-bearing services (postgres, mysql, redis), errors if the
// declared version isn't supported or installed. For single-version services
// (mailpit, rustfs), errors if the service isn't installed — pv.yml should
// fail loud, never silently bind a service that won't be there.
type ApplyPvYmlServicesStep struct{}

var _ automation.Step = (*ApplyPvYmlServicesStep)(nil)

func (s *ApplyPvYmlServicesStep) Label() string  { return "Bind services from pv.yml" }
func (s *ApplyPvYmlServicesStep) Gate() string   { return "apply_pvyml_services" }
func (s *ApplyPvYmlServicesStep) Critical() bool { return true }
func (s *ApplyPvYmlServicesStep) Verbose() bool  { return false }

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
		if !mysql.IsInstalled(version) {
			return "", fmt.Errorf("pv.yml mysql %q is not installed — run `pv mysql:install %s`", version, version)
		}
		bindProjectMysql(ctx.Registry, ctx.ProjectName, version)
		count++
	}

	if cfg.Redis != nil {
		version, err := redis.ResolveVersion(cfg.Redis.Version)
		if err != nil {
			return "", err
		}
		if !redis.IsInstalled(version) {
			return "", fmt.Errorf("pv.yml redis %s is not installed — run `pv redis:install %s`", version, version)
		}
		bindProjectRedis(ctx.Registry, ctx.ProjectName, version)
		count++
	}

	if cfg.Mailpit != nil {
		version, err := mailpit.ResolveVersion(cfg.Mailpit.Version)
		if err != nil {
			return "", err
		}
		if !mailpit.IsInstalled(version) {
			return "", fmt.Errorf("pv.yml mailpit %s is not installed - run `pv mailpit:install %s`", version, version)
		}
		bindProjectMail(ctx.Registry, ctx.ProjectName, version)
		count++
	}

	if cfg.Rustfs != nil {
		version, err := rustfs.ResolveVersion(cfg.Rustfs.Version)
		if err != nil {
			return "", err
		}
		if !rustfs.IsInstalled(version) {
			return "", fmt.Errorf("pv.yml rustfs %s is not installed - run `pv rustfs:install %s`", version, version)
		}
		bindProjectS3(ctx.Registry, ctx.ProjectName, version)
		count++
	}

	return fmt.Sprintf("bound %d service(s) from pv.yml", count), nil
}

func bindProjectMail(reg *registry.Registry, projectName, version string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		reg.Projects[i].Services.Mail = version
		return
	}
}

func bindProjectS3(reg *registry.Registry, projectName, version string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		reg.Projects[i].Services.S3 = version
		return
	}
}

func bindProjectRedis(reg *registry.Registry, projectName, version string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		reg.Projects[i].Services.Redis = version
		return
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

func bindProjectMysql(reg *registry.Registry, projectName, version string) {
	for i := range reg.Projects {
		if reg.Projects[i].Name != projectName {
			continue
		}
		if reg.Projects[i].Services == nil {
			reg.Projects[i].Services = &registry.ProjectServices{}
		}
		reg.Projects[i].Services.MySQL = version
		return
	}
}
