package mailpit

import (
	"reflect"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/caddy"
)

func TestServiceKey(t *testing.T) {
	if ServiceKey() != "mail" {
		t.Errorf("ServiceKey() = %q, want mail", ServiceKey())
	}
}

func TestPorts(t *testing.T) {
	if Port() != 1025 {
		t.Errorf("Port() = %d, want 1025", Port())
	}
	if ConsolePort() != 8025 {
		t.Errorf("ConsolePort() = %d, want 8025", ConsolePort())
	}
}

func TestWebRoutes(t *testing.T) {
	want := []caddy.WebRoute{
		{Subdomain: "mail", Port: 8025},
	}
	got := WebRoutes()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("WebRoutes() = %#v, want %#v", got, want)
	}
}

// TestMailpit_EnvVars_Golden is pinned against the exact keys/values the
// old Docker Mail service produced so linked projects do not need .env
// rewrites post-migration.
func TestMailpit_EnvVars_Golden(t *testing.T) {
	got := EnvVars("anyproject")
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

// TestBuildSupervisorProcess_PinsBindAddresses locks the --smtp / --listen
// values to :1025 and :8025 so a future edit to one side (Args or
// Port/ConsolePort) cannot silently drift the other. Port/Args agreement
// is load-bearing: the EnvVars golden map pins MAIL_PORT=1025 against
// the SMTP flag, and the Caddy WebRoute for mail.pv.{tld} points at 8025
// from the --listen flag.
func TestBuildSupervisorProcess_PinsBindAddresses(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	want := map[string]string{
		"--smtp":   ":1025",
		"--listen": ":8025",
	}
	for flag, value := range want {
		found := false
		for i := 0; i < len(proc.Args)-1; i++ {
			if proc.Args[i] == flag && proc.Args[i+1] == value {
				found = true
				break
			}
		}
		if !found {
			t.Errorf("Args() missing %q %q; got %v", flag, value, proc.Args)
		}
	}
}

func TestBuildSupervisorProcess_DatabaseUsesDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	var dbValue string
	for i := 0; i < len(proc.Args)-1; i++ {
		if proc.Args[i] == "--database" {
			dbValue = proc.Args[i+1]
			break
		}
	}
	if dbValue == "" {
		t.Fatalf("Args missing --database; got %v", proc.Args)
	}
	if !strings.HasSuffix(dbValue, "/mailpit.db") {
		t.Errorf("--database value should end with /mailpit.db; got %q", dbValue)
	}
}

func TestBuildSupervisorProcess_NoEnv(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	if proc.Env != nil {
		t.Errorf("Env should be nil (mailpit takes no env vars); got %v", proc.Env)
	}
}

func TestBuildSupervisorProcess_ReadyTimeoutSet(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	if proc.ReadyTimeout == 0 {
		t.Error("ReadyTimeout must be non-zero")
	}
	if proc.Ready == nil {
		t.Error("Ready func must be set")
	}
}
