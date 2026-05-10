package mailpit

import (
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
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

// TestBuildSupervisorProcess_NameAndPaths locks the process name, binary path,
// and log file path so that a future rename of ServiceKey() or Binary().Name
// cannot silently route to the wrong supervisor map entry. The supervisor map
// in manager.go is keyed by proc.Name (= "mailpit"), NOT by ServiceKey()
// (= "mail"). A swap of the two would compile cleanly but break supervision
// because the map lookup would miss.
func TestBuildSupervisorProcess_NameAndPaths(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}

	// proc.Name must be "mailpit" (the binary name), NOT "mail" (the service key).
	// The supervisor manager keyes its process map off proc.Name; using "mail"
	// here would silently route to the wrong entry.
	if proc.Name != "mailpit" {
		t.Errorf("proc.Name = %q, want %q (not the service key %q)", proc.Name, "mailpit", "mail")
	}

	wantBinarySuffix := "/.pv/internal/bin/mailpit"
	if !strings.HasSuffix(proc.Binary, wantBinarySuffix) {
		t.Errorf("proc.Binary = %q, want suffix %q", proc.Binary, wantBinarySuffix)
	}

	wantLogSuffix := "/.pv/logs/mailpit.log"
	if !strings.HasSuffix(proc.LogFile, wantLogSuffix) {
		t.Errorf("proc.LogFile = %q, want suffix %q", proc.LogFile, wantLogSuffix)
	}

	dataDir := config.ServiceDataDir("mail", "latest")
	info, err := os.Stat(dataDir)
	if err != nil {
		t.Errorf("data dir %q was not created: %v", dataDir, err)
	} else if !info.IsDir() {
		t.Errorf("data dir %q exists but is not a directory", dataDir)
	}

	logDir := config.LogsDir()
	info, err = os.Stat(logDir)
	if err != nil {
		t.Errorf("log dir %q was not created: %v", logDir, err)
	} else if !info.IsDir() {
		t.Errorf("log dir %q exists but is not a directory", logDir)
	}
}
