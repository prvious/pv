package services

import (
	"time"

	"github.com/prvious/pv/internal/binaries"
)

type Mailpit struct{}

func (m *Mailpit) Name() string        { return "mail" }
func (m *Mailpit) DisplayName() string { return "Mail (Mailpit)" }

func (m *Mailpit) Binary() binaries.Binary { return binaries.Mailpit }

func (m *Mailpit) Args(dataDir string) []string {
	// Flag names verified in Task 1; adjust here if reality differs.
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

func (m *Mailpit) ReadyCheck() ReadyCheck {
	return ReadyCheck{
		HTTPEndpoint: "http://127.0.0.1:8025/livez",
		Timeout:      30 * time.Second,
	}
}

func init() {
	binaryRegistry["mail"] = &Mailpit{}
}
