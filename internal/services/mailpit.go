package services

import (
	"time"

	"github.com/prvious/pv/internal/binaries"
)

// Mailpit is a BinaryService wrapper around the upstream Mailpit binary.
// Name() returns "mail" (not "mailpit") to preserve the service key used by
// the previous Docker-based mail service — renaming breaks existing pv.yml
// references and linked projects' .env files. See TestMailpit_EnvVars_Golden
// for the migration contract with the old Docker service.
type Mailpit struct{}

func (m *Mailpit) Name() string        { return "mail" }
func (m *Mailpit) DisplayName() string { return "Mail (Mailpit)" }

func (m *Mailpit) Binary() binaries.Binary { return binaries.Mailpit }

// Args pins the SMTP and HTTP bind addresses to :1025 and :8025; the values
// must agree with Port() and ConsolePort() or MAIL_PORT / WebRoutes drift.
// Flag names match `mailpit --help` for v1.29.6.
func (m *Mailpit) Args(dataDir string) []string {
	return []string{
		"--smtp", ":1025",
		"--listen", ":8025",
		"--database", dataDir + "/mailpit.db",
	}
}

func (m *Mailpit) Env() []string { return nil }

func (m *Mailpit) Port() int        { return 1025 }
func (m *Mailpit) ConsolePort() int { return 8025 }

func (m *Mailpit) WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "mail", Port: 8025},
	}
}

func (m *Mailpit) EnvVars(_ string) map[string]string {
	return map[string]string{
		"MAIL_MAILER":   "smtp",
		"MAIL_HOST":     "127.0.0.1",
		"MAIL_PORT":     "1025",
		"MAIL_USERNAME": "",
		"MAIL_PASSWORD": "",
	}
}

// ReadyCheck uses Mailpit's documented /livez endpoint, which returns 200 once
// both the SMTP and HTTP servers are listening.
func (m *Mailpit) ReadyCheck() ReadyCheck {
	return ReadyCheck{
		HTTPEndpoint: "http://127.0.0.1:8025/livez",
		Timeout:      30 * time.Second,
	}
}

// Self-registers on import; see binaryRegistry in binary.go for the pattern.
func init() {
	binaryRegistry["mail"] = &Mailpit{}
}
