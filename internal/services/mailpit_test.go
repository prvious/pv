package services

import (
	"reflect"
	"testing"
)

func TestMailpit_RegisteredAsMail(t *testing.T) {
	svc, ok := LookupBinary("mail")
	if !ok {
		t.Fatal("LookupBinary(\"mail\") returned ok=false")
	}
	if _, isMailpit := svc.(*Mailpit); !isMailpit {
		t.Errorf("expected *Mailpit, got %T", svc)
	}
}

func TestMailpit_Name(t *testing.T) {
	m := &Mailpit{}
	if m.Name() != "mail" {
		t.Errorf("Name() = %q, want mail", m.Name())
	}
}

func TestMailpit_Ports(t *testing.T) {
	m := &Mailpit{}
	if m.Port() != 1025 {
		t.Errorf("Port() = %d, want 1025", m.Port())
	}
	if m.ConsolePort() != 8025 {
		t.Errorf("ConsolePort() = %d, want 8025", m.ConsolePort())
	}
}

func TestMailpit_WebRoutes(t *testing.T) {
	m := &Mailpit{}
	want := []WebRoute{
		{Subdomain: "mail", Port: 8025},
	}
	got := m.WebRoutes()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("WebRoutes() = %#v, want %#v", got, want)
	}
}

func TestMailpit_EnvVars_Golden(t *testing.T) {
	// Pinned against the exact keys/values the old Docker Mail service
	// produced so linked projects do not need .env rewrites post-migration.
	m := &Mailpit{}
	got := m.EnvVars("anyproject")
	want := map[string]string{
		"MAIL_MAILER":   "smtp",
		"MAIL_HOST":     "127.0.0.1",
		"MAIL_PORT":     "1025",
		"MAIL_USERNAME": "",
		"MAIL_PASSWORD": "",
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("EnvVars() = %#v, want %#v", got, want)
	}
}

func TestMailpit_Args_UsesDataDir(t *testing.T) {
	m := &Mailpit{}
	args := m.Args("/tmp/mailpit-data")
	found := false
	for _, a := range args {
		if a == "/tmp/mailpit-data" || a == "/tmp/mailpit-data/mailpit.db" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("Args() did not include the data dir; got %v", args)
	}
}

// TestMailpit_Args_PinsBindAddresses locks the --smtp / --listen values to
// :1025 and :8025 so a future edit to one side (Args or Port/ConsolePort) can
// not silently drift the other. Port/Args agreement is load-bearing: the
// EnvVars golden map pins MAIL_PORT=1025 against the SMTP flag, and the Caddy
// WebRoute for mail.pv.{tld} points at 8025 from the --listen flag.
func TestMailpit_Args_PinsBindAddresses(t *testing.T) {
	m := &Mailpit{}
	args := m.Args("/tmp/mailpit-data")
	want := map[string]string{
		"--smtp":     ":1025",
		"--listen":   ":8025",
		"--database": "/tmp/mailpit-data/mailpit.db",
	}
	for flag, value := range want {
		found := false
		for i := 0; i < len(args)-1; i++ {
			if args[i] == flag && args[i+1] == value {
				found = true
				break
			}
		}
		if !found {
			t.Errorf("Args() missing %q %q; got %v", flag, value, args)
		}
	}
}

func TestMailpit_ReadyCheck_HTTPLivez(t *testing.T) {
	m := &Mailpit{}
	rc := m.ReadyCheck()
	if rc.HTTPEndpoint() == "" {
		t.Error("ReadyCheck.HTTPEndpoint must be set (Mailpit uses HTTP probe, not TCP)")
	}
	if rc.TCPPort() != 0 {
		t.Errorf("ReadyCheck.TCPPort = %d, want 0", rc.TCPPort())
	}
	if rc.Timeout == 0 {
		t.Error("ReadyCheck.Timeout must be non-zero")
	}
}
