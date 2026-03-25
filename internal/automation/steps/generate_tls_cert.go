package steps

import (
	"fmt"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/certs"
)

// GenerateTLSCertStep generates a TLS certificate for the project hostname.
type GenerateTLSCertStep struct{}

var _ automation.Step = (*GenerateTLSCertStep)(nil)

func (s *GenerateTLSCertStep) Label() string  { return "Generate TLS certificate" }
func (s *GenerateTLSCertStep) Gate() string   { return "generate_tls_cert" }
func (s *GenerateTLSCertStep) Critical() bool { return false }

func (s *GenerateTLSCertStep) ShouldRun(_ *automation.Context) bool {
	return true
}

func (s *GenerateTLSCertStep) Run(ctx *automation.Context) (string, error) {
	hostname := fmt.Sprintf("%s.%s", ctx.ProjectName, ctx.TLD)
	if err := certs.EnsureValetConfig(ctx.TLD); err != nil {
		return "", fmt.Errorf("TLS cert setup skipped: %w", err)
	}
	if err := certs.GenerateSiteTLS(hostname); err != nil {
		return "", fmt.Errorf("TLS cert not generated for %s: %w", hostname, err)
	}
	return hostname, nil
}
