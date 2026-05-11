package laravel

import (
	"fmt"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/certs"
	"github.com/prvious/pv/internal/projectenv"
)

// isLaravel returns true if the project type is Laravel or Laravel with Octane.
func isLaravel(projectType string) bool {
	return projectType == "laravel" || projectType == "laravel-octane"
}

// --- SetAppURLStep ---

// SetAppURLStep sets APP_URL in .env to https://{name}.{tld}.
type SetAppURLStep struct{}

var _ automation.Step = (*SetAppURLStep)(nil)

func (s *SetAppURLStep) Label() string  { return "Set APP_URL" }
func (s *SetAppURLStep) Gate() string   { return "set_app_url" }
func (s *SetAppURLStep) Critical() bool { return false }
func (s *SetAppURLStep) Verbose() bool  { return false }

func (s *SetAppURLStep) ShouldRun(ctx *automation.Context) bool {
	return isLaravel(ctx.ProjectType) && HasEnvFile(ctx.ProjectPath)
}

func (s *SetAppURLStep) Run(ctx *automation.Context) (string, error) {
	tld := ctx.TLD
	if tld == "" {
		tld = "test"
	}
	appURL := fmt.Sprintf("https://%s.%s", ctx.ProjectName, tld)
	envPath := filepath.Join(ctx.ProjectPath, ".env")
	vars := map[string]string{"APP_URL": appURL}
	if err := projectenv.MergeDotEnv(envPath, "", vars); err != nil {
		return "", fmt.Errorf("set APP_URL: %w", err)
	}
	return appURL, nil
}

// --- SetViteTLSStep ---

// SetViteTLSStep sets VITE_DEV_SERVER_KEY and VITE_DEV_SERVER_CERT in .env
// so laravel-vite-plugin can find the TLS certificate for the dev server.
type SetViteTLSStep struct{}

var _ automation.Step = (*SetViteTLSStep)(nil)

func (s *SetViteTLSStep) Label() string  { return "Set Vite TLS" }
func (s *SetViteTLSStep) Gate() string   { return "set_vite_tls" }
func (s *SetViteTLSStep) Critical() bool { return false }
func (s *SetViteTLSStep) Verbose() bool  { return false }

func (s *SetViteTLSStep) ShouldRun(ctx *automation.Context) bool {
	return isLaravel(ctx.ProjectType) && HasEnvFile(ctx.ProjectPath)
}

func (s *SetViteTLSStep) Run(ctx *automation.Context) (string, error) {
	tld := ctx.TLD
	if tld == "" {
		tld = "test"
	}
	hostname := fmt.Sprintf("%s.%s", ctx.ProjectName, tld)
	envPath := filepath.Join(ctx.ProjectPath, ".env")
	vars := map[string]string{
		"VITE_DEV_SERVER_CERT": certs.CertPath(hostname),
		"VITE_DEV_SERVER_KEY":  certs.KeyPath(hostname),
	}
	if err := projectenv.MergeDotEnv(envPath, "", vars); err != nil {
		return "", fmt.Errorf("set Vite TLS: %w", err)
	}
	return hostname, nil
}
