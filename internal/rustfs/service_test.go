package rustfs

import (
	"reflect"
	"testing"

	"github.com/prvious/pv/internal/caddy"
)

func TestServiceKey(t *testing.T) {
	if ServiceKey() != "s3" {
		t.Errorf("ServiceKey() = %q, want s3", ServiceKey())
	}
}

func TestPorts(t *testing.T) {
	if Port() != 9000 {
		t.Errorf("Port() = %d, want 9000", Port())
	}
	if ConsolePort() != 9001 {
		t.Errorf("ConsolePort() = %d, want 9001", ConsolePort())
	}
}

func TestWebRoutes(t *testing.T) {
	want := []caddy.WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
	got := WebRoutes()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("WebRoutes() = %#v, want %#v", got, want)
	}
}

// TestEnvVars_MatchesDockerKeys: linked projects rely on these exact .env
// keys; the binary migration must not silently change them.
func TestEnvVars_MatchesDockerKeys(t *testing.T) {
	vars := EnvVars("myproject")
	wantKeys := []string{
		"AWS_ACCESS_KEY_ID",
		"AWS_SECRET_ACCESS_KEY",
		"AWS_DEFAULT_REGION",
		"AWS_BUCKET",
		"AWS_ENDPOINT",
		"AWS_USE_PATH_STYLE_ENDPOINT",
	}
	for _, k := range wantKeys {
		if _, ok := vars[k]; !ok {
			t.Errorf("EnvVars missing key %q", k)
		}
	}
	if vars["AWS_BUCKET"] != "myproject" {
		t.Errorf("AWS_BUCKET = %q, want myproject", vars["AWS_BUCKET"])
	}
	if vars["AWS_ENDPOINT"] != "http://127.0.0.1:9000" {
		t.Errorf("AWS_ENDPOINT = %q, want http://127.0.0.1:9000", vars["AWS_ENDPOINT"])
	}
}

func TestBuildSupervisorProcess_IncludesDataDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	// The data dir is the second positional arg after "server".
	if len(proc.Args) < 2 || proc.Args[0] != "server" {
		t.Fatalf("Args do not start with `server <dataDir>`: %v", proc.Args)
	}
	if proc.Args[1] == "" {
		t.Errorf("data dir arg is empty; got %v", proc.Args)
	}
}

// TestBuildSupervisorProcess_IncludesConsoleEnable: verified 2026-04-14
// — port 9001 does not bind unless --console-enable is passed
// explicitly.
func TestBuildSupervisorProcess_IncludesConsoleEnable(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	found := false
	for _, a := range proc.Args {
		if a == "--console-enable" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("Args missing --console-enable; got %v", proc.Args)
	}
}

func TestBuildSupervisorProcess_EnvUsesAccessSecretKeys(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess()
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}
	var sawAccess, sawSecret bool
	for _, e := range proc.Env {
		if e == "RUSTFS_ACCESS_KEY=rstfsadmin" {
			sawAccess = true
		}
		if e == "RUSTFS_SECRET_KEY=rstfsadmin" {
			sawSecret = true
		}
	}
	if !sawAccess {
		t.Errorf("Env missing RUSTFS_ACCESS_KEY; got %v", proc.Env)
	}
	if !sawSecret {
		t.Errorf("Env missing RUSTFS_SECRET_KEY; got %v", proc.Env)
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
