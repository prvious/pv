package steps

import (
	"fmt"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/redis"
	"github.com/prvious/pv/internal/rustfs"
)

// ApplyPvYmlEnvStep renders pv.yml's top-level env: and per-service
// env: templates against their respective variable maps and merges
// the rendered keys into the project's .env with pv-managed markers.
//
// Runs when ctx.ProjectConfig.HasAnyEnv(). For version-bearing
// services (postgres/mysql), probes the installed binary to populate
// the .version / .dsn template vars; the previous step
// (ApplyPvYmlServicesStep) guarantees the binary is installed.
type ApplyPvYmlEnvStep struct{}

var _ automation.Step = (*ApplyPvYmlEnvStep)(nil)

func (s *ApplyPvYmlEnvStep) Label() string  { return "Apply pv.yml env templates" }
func (s *ApplyPvYmlEnvStep) Gate() string   { return "apply_pvyml_env" }
func (s *ApplyPvYmlEnvStep) Critical() bool { return true }
func (s *ApplyPvYmlEnvStep) Verbose() bool  { return false }

func (s *ApplyPvYmlEnvStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.ProjectConfig.HasAnyEnv()
}

func (s *ApplyPvYmlEnvStep) Run(ctx *automation.Context) (string, error) {
	cfg := ctx.ProjectConfig
	rendered := map[string]string{}

	// Top-level env: project-level vars.
	if len(cfg.Env) > 0 {
		vars := projectenv.ProjectTemplateVars(ctx.ProjectName, ctx.TLD)
		if err := renderIntoMap(rendered, cfg.Env, vars, "env"); err != nil {
			return "", err
		}
	}

	// postgresql.env
	if cfg.Postgresql != nil && len(cfg.Postgresql.Env) > 0 {
		full, err := postgres.ProbeVersion(cfg.Postgresql.Version)
		if err != nil {
			return "", fmt.Errorf("probe postgres %q: %w", cfg.Postgresql.Version, err)
		}
		vars, err := postgres.TemplateVars(cfg.Postgresql.Version, full)
		if err != nil {
			return "", fmt.Errorf("postgres template vars: %w", err)
		}
		if err := renderIntoMap(rendered, cfg.Postgresql.Env, vars, "postgresql.env"); err != nil {
			return "", err
		}
	}

	// mysql.env
	if cfg.Mysql != nil && len(cfg.Mysql.Env) > 0 {
		full, err := mysql.ProbeVersion(cfg.Mysql.Version)
		if err != nil {
			return "", fmt.Errorf("probe mysql %q: %w", cfg.Mysql.Version, err)
		}
		vars, err := mysql.TemplateVars(cfg.Mysql.Version, full)
		if err != nil {
			return "", fmt.Errorf("mysql template vars: %w", err)
		}
		if err := renderIntoMap(rendered, cfg.Mysql.Env, vars, "mysql.env"); err != nil {
			return "", err
		}
	}

	// redis.env
	if cfg.Redis != nil && len(cfg.Redis.Env) > 0 {
		version, err := redis.ResolveVersion(cfg.Redis.Version)
		if err != nil {
			return "", err
		}
		if err := renderIntoMap(rendered, cfg.Redis.Env, redis.TemplateVars(version), "redis.env"); err != nil {
			return "", err
		}
	}

	// mailpit.env
	if cfg.Mailpit != nil && len(cfg.Mailpit.Env) > 0 {
		if err := renderIntoMap(rendered, cfg.Mailpit.Env, mailpit.TemplateVars(), "mailpit.env"); err != nil {
			return "", err
		}
	}

	// rustfs.env
	if cfg.Rustfs != nil && len(cfg.Rustfs.Env) > 0 {
		if err := renderIntoMap(rendered, cfg.Rustfs.Env, rustfs.TemplateVars(rustfs.DefaultVersion()), "rustfs.env"); err != nil {
			return "", err
		}
	}

	envPath := filepath.Join(ctx.ProjectPath, ".env")
	backupPath := filepath.Join(ctx.ProjectPath, ".pv-backup")
	if err := projectenv.MergeManagedDotEnv(envPath, backupPath, rendered); err != nil {
		return "", fmt.Errorf("merge .env: %w", err)
	}
	return fmt.Sprintf("wrote %d key(s) to .env", len(rendered)), nil
}

// renderIntoMap renders each template in src against vars and accumulates
// the result into dst. scope is used only for error messages.
// Returns an error if a key already exists in dst (duplicate across scopes).
func renderIntoMap(dst, src, vars map[string]string, scope string) error {
	for key, tmpl := range src {
		if _, exists := dst[key]; exists {
			return fmt.Errorf("%s[%s]: duplicate env key across scopes", scope, key)
		}
		out, err := projectenv.Render(tmpl, vars)
		if err != nil {
			return fmt.Errorf("%s[%s]: %w", scope, key, err)
		}
		dst[key] = out
	}
	return nil
}
