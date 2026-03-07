package services

import (
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
)

type Mail struct{}

func (m *Mail) Name() string        { return "mail" }
func (m *Mail) DisplayName() string { return "Mail" }

func (m *Mail) DefaultVersion() string { return "latest" }

func (m *Mail) ImageName(version string) string {
	return "axllent/mailpit:" + version
}

func (m *Mail) ContainerName(version string) string {
	return "pv-mail-" + version
}

func (m *Mail) Port(_ string) int        { return 1025 }
func (m *Mail) ConsolePort(_ string) int { return 8025 }

func (m *Mail) WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "mail", Port: 8025},
	}
}

func (m *Mail) CreateOpts(version string) container.CreateOpts {
	return container.CreateOpts{
		Name:  m.ContainerName(version),
		Image: m.ImageName(version),
		Ports: map[int]int{
			1025: 1025,
			8025: 8025,
		},
		Volumes: map[string]string{
			config.ServiceDataDir("mail", version): "/data",
		},
		Labels: map[string]string{
			"dev.prvious.pv":         "true",
			"dev.prvious.pv.service": "mail",
			"dev.prvious.pv.version": version,
		},
		HealthCmd:      []string{"CMD-SHELL", "wget -q --spider http://localhost:8025/livez || exit 1"},
		HealthInterval: "2s",
		HealthTimeout:  "5s",
		HealthRetries:  15,
	}
}

func (m *Mail) EnvVars(_ string, _ int) map[string]string {
	return map[string]string{
		"MAIL_MAILER":   "smtp",
		"MAIL_HOST":     "127.0.0.1",
		"MAIL_PORT":     "1025",
		"MAIL_USERNAME": "",
		"MAIL_PASSWORD": "",
	}
}

func (m *Mail) CreateDatabase(_ *container.Engine, _, _ string) error {
	return nil
}

func (m *Mail) HasDatabases() bool { return false }
