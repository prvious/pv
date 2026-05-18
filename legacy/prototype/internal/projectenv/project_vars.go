package projectenv

import (
	"fmt"

	"github.com/prvious/pv/internal/certs"
)

// ProjectTemplateVars returns the template variables available at the
// top-level `env:` block in a pv.yml. projectName should already be
// sanitized by SanitizeProjectName; tld is the resolved per-machine
// TLD (e.g., "test").
func ProjectTemplateVars(projectName, tld string) map[string]string {
	host := fmt.Sprintf("%s.%s", projectName, tld)
	return map[string]string{
		"site_url":      "https://" + host,
		"site_host":     host,
		"tls_cert_path": certs.CertPath(host),
		"tls_key_path":  certs.KeyPath(host),
	}
}
