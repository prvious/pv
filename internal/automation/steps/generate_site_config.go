package steps

import (
	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/registry"
)

// GenerateSiteConfigStep writes the per-site Caddy config file.
type GenerateSiteConfigStep struct{}

var _ automation.Step = (*GenerateSiteConfigStep)(nil)

func (s *GenerateSiteConfigStep) Label() string  { return "Generate site config" }
func (s *GenerateSiteConfigStep) Gate() string   { return "generate_site_config" }
func (s *GenerateSiteConfigStep) Critical() bool { return true }

func (s *GenerateSiteConfigStep) ShouldRun(_ *automation.Context) bool {
	return true
}

func (s *GenerateSiteConfigStep) Run(ctx *automation.Context) (string, error) {
	project := registry.Project{
		Name: ctx.ProjectName,
		Path: ctx.ProjectPath,
		Type: ctx.ProjectType,
		PHP:  ctx.PHPVersion,
	}
	if err := caddy.GenerateSiteConfig(project, ctx.GlobalPHP); err != nil {
		return "", err
	}
	return ctx.ProjectName + ".caddy", nil
}
