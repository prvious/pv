package steps

import (
	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/caddy"
)

// GenerateCaddyfileStep regenerates the root Caddyfile that imports all site configs.
type GenerateCaddyfileStep struct{}

var _ automation.Step = (*GenerateCaddyfileStep)(nil)

func (s *GenerateCaddyfileStep) Label() string  { return "Generate Caddyfile" }
func (s *GenerateCaddyfileStep) Gate() string   { return "generate_caddyfile" }
func (s *GenerateCaddyfileStep) Critical() bool { return true }
func (s *GenerateCaddyfileStep) Verbose() bool  { return false }

func (s *GenerateCaddyfileStep) ShouldRun(_ *automation.Context) bool {
	return true
}

func (s *GenerateCaddyfileStep) Run(_ *automation.Context) (string, error) {
	if err := caddy.GenerateCaddyfile(); err != nil {
		return "", err
	}
	return "Caddyfile updated", nil
}
