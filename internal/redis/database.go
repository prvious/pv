package redis

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/registry"
)

// EnvWriter is the per-project .env writer. Wired at init time by the
// cobra layer (internal/commands/redis/install.go) to call
// laravel.UpdateProjectEnvForRedis. Kept as a callback to break the
// internal/redis ↔ internal/laravel import cycle (laravel imports redis
// for EnvVars; redis can't import laravel back).
//
// Signature parallels the existing laravel.UpdateProjectEnvFor*
// helpers: (projectPath, projectName, *ProjectServices) error.
var EnvWriter func(projectPath, projectName string, bound *registry.ProjectServices) error

// BindLinkedProjects walks the registry and binds every Laravel-shaped
// project to redis (Services.Redis = true) plus, when EnvWriter is
// wired, writes REDIS_HOST/PORT/PASSWORD to each project's .env file.
//
// Mirrors mailpit/rustfs single-version auto-bind: redis is a
// transparent dependency for Laravel apps, so we don't gate on the
// project's existing .env content (no DB_CONNECTION-style heuristic).
//
// Saves the registry once at the end if anything changed.
func BindLinkedProjects() error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("load registry: %w", err)
	}
	changed := false
	for i := range reg.Projects {
		p := &reg.Projects[i]
		if p.Type != "laravel" && p.Type != "laravel-octane" {
			continue
		}
		if p.Services == nil {
			p.Services = &registry.ProjectServices{}
		}
		if !p.Services.Redis {
			p.Services.Redis = true
			changed = true
		}
		if EnvWriter != nil {
			if err := EnvWriter(p.Path, p.Name, p.Services); err != nil {
				// Best-effort: don't fail the whole install on one
				// project's .env write. Stderr keeps stdout
				// machine-readable per CLAUDE.md UI rules.
				fmt.Fprintf(os.Stderr, "redis: bind %s: %v\n", p.Name, err)
			}
		}
	}
	if changed {
		if err := reg.Save(); err != nil {
			return fmt.Errorf("save registry: %w", err)
		}
	}
	return nil
}
