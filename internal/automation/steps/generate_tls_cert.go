package steps

import (
	"fmt"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/certs"
)

// GenerateTLSCertStep generates a TLS certificate for the project hostname
// and any aliases declared in pv.yml.
type GenerateTLSCertStep struct{}

var _ automation.Step = (*GenerateTLSCertStep)(nil)

func (s *GenerateTLSCertStep) Label() string  { return "Generate TLS certificate" }
func (s *GenerateTLSCertStep) Gate() string   { return "generate_tls_cert" }
func (s *GenerateTLSCertStep) Critical() bool { return false }
func (s *GenerateTLSCertStep) Verbose() bool  { return false }

func (s *GenerateTLSCertStep) ShouldRun(_ *automation.Context) bool {
	return true
}

func (s *GenerateTLSCertStep) Run(ctx *automation.Context) (string, error) {
	var aliases []string
	if ctx.ProjectConfig != nil {
		aliases = ctx.ProjectConfig.Aliases
	}
	hosts := expandHostsForCertMinting(ctx.ProjectName, ctx.TLD, aliases)
	for _, h := range hosts {
		if err := certs.GenerateSiteTLS(h); err != nil {
			return "", fmt.Errorf("TLS cert not generated for %s: %w", h, err)
		}
	}
	if len(hosts) == 1 {
		return hosts[0], nil
	}
	noun := "alias"
	if len(hosts) > 2 {
		noun = "aliases"
	}
	return fmt.Sprintf("%s (+%d %s)", hosts[0], len(hosts)-1, noun), nil
}

// expandHostsForCertMinting returns the primary host followed by every
// alias, in order. Aliases are taken verbatim — pv.yml authors write
// fully-qualified hostnames (e.g., "admin.myapp.test").
func expandHostsForCertMinting(project, tld string, aliases []string) []string {
	hosts := make([]string, 0, 1+len(aliases))
	hosts = append(hosts, fmt.Sprintf("%s.%s", project, tld))
	hosts = append(hosts, aliases...)
	return hosts
}
