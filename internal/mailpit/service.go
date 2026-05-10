package mailpit

import (
	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	mailpitproc "github.com/prvious/pv/internal/mailpit/proc"
	"github.com/prvious/pv/internal/supervisor"
)

const (
	displayName = "Mail (Mailpit)"
	// serviceKey must remain "mail" (not "mailpit") to preserve compatibility
	// with the previous Docker-based mail service — renaming breaks existing
	// pv.yml references and linked projects' .env files. See
	// TestMailpit_EnvVars_Golden for the migration contract with the old
	// Docker service.
	serviceKey  = "mail"
	port        = 1025
	consolePort = 8025
)

// Binary returns the binaries.Binary descriptor for mailpit.
// Delegates to the leaf proc package so that internal/server can import
// proc directly without creating an import cycle through this package.
func Binary() binaries.Binary { return mailpitproc.Binary() }

func Port() int           { return port }
func ConsolePort() int    { return consolePort }
func DisplayName() string { return displayName }
func ServiceKey() string  { return serviceKey }

func WebRoutes() []caddy.WebRoute {
	raw := mailpitproc.WebRoutes()
	out := make([]caddy.WebRoute, len(raw))
	for i, r := range raw {
		out[i] = caddy.WebRoute{Subdomain: r.Subdomain, Port: r.Port}
	}
	return out
}

func EnvVars(_ string) map[string]string {
	return map[string]string{
		"MAIL_MAILER":   "smtp",
		"MAIL_HOST":     "127.0.0.1",
		"MAIL_PORT":     "1025",
		"MAIL_USERNAME": "",
		"MAIL_PASSWORD": "",
	}
}

// BuildSupervisorProcess returns the supervisor.Process for mailpit.
// Delegates to proc.BuildSupervisorProcess so the build logic is defined
// once in the leaf package.
func BuildSupervisorProcess() (supervisor.Process, error) {
	return mailpitproc.BuildSupervisorProcess()
}
