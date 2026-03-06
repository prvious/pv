package services

import "testing"

func TestMailPorts(t *testing.T) {
	m := &Mail{}
	if got := m.Port("latest"); got != 1025 {
		t.Errorf("Port = %d, want 1025", got)
	}
	if got := m.ConsolePort("latest"); got != 8025 {
		t.Errorf("ConsolePort = %d, want 8025", got)
	}
}

func TestMailImageName(t *testing.T) {
	m := &Mail{}
	if got := m.ImageName("latest"); got != "axllent/mailpit:latest" {
		t.Errorf("ImageName = %q, want %q", got, "axllent/mailpit:latest")
	}
}

func TestMailEnvVars(t *testing.T) {
	m := &Mail{}
	env := m.EnvVars("", 0)
	if env["MAIL_MAILER"] != "smtp" {
		t.Errorf("MAIL_MAILER = %q", env["MAIL_MAILER"])
	}
	if env["MAIL_HOST"] != "127.0.0.1" {
		t.Errorf("MAIL_HOST = %q", env["MAIL_HOST"])
	}
	if env["MAIL_PORT"] != "1025" {
		t.Errorf("MAIL_PORT = %q", env["MAIL_PORT"])
	}
}

func TestMailWebRoutes(t *testing.T) {
	m := &Mail{}
	routes := m.WebRoutes()
	if len(routes) != 1 {
		t.Fatalf("WebRoutes len = %d, want 1", len(routes))
	}
	if routes[0].Subdomain != "mail" || routes[0].Port != 8025 {
		t.Errorf("route[0] = %+v, want {mail 8025}", routes[0])
	}
}

func TestMailName(t *testing.T) {
	m := &Mail{}
	if m.Name() != "mail" {
		t.Errorf("Name = %q, want %q", m.Name(), "mail")
	}
}
