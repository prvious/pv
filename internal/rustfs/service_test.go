package rustfs

import (
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
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
	want := []WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
	got := WebRoutes()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("WebRoutes() = %#v, want %#v", got, want)
	}
}

// TestEnvVars_MatchesLaravelS3Keys: linked projects rely on these exact .env
// keys when RustFS is bound through pv.yml.
func TestEnvVars_MatchesLaravelS3Keys(t *testing.T) {
	vars, err := EnvVars(DefaultVersion(), "myproject")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
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
	proc, err := BuildSupervisorProcess(DefaultVersion())
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
	proc, err := BuildSupervisorProcess(DefaultVersion())
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
	proc, err := BuildSupervisorProcess(DefaultVersion())
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
	proc, err := BuildSupervisorProcess(DefaultVersion())
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
// in manager.go is keyed by proc.Name, not by ServiceKey(); a swap of the two
// would compile cleanly but break supervision.
func TestBuildSupervisorProcess_NameAndPaths(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess(DefaultVersion())
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}

	if proc.Name != "rustfs-latest" {
		t.Errorf("proc.Name = %q, want %q", proc.Name, "rustfs-latest")
	}

	wantBinarySuffix := "/.pv/internal/bin/rustfs"
	if !strings.HasSuffix(proc.Binary, wantBinarySuffix) {
		t.Errorf("proc.Binary = %q, want suffix %q", proc.Binary, wantBinarySuffix)
	}

	wantLogSuffix := "/.pv/logs/rustfs-latest.log"
	if !strings.HasSuffix(proc.LogFile, wantLogSuffix) {
		t.Errorf("proc.LogFile = %q, want suffix %q", proc.LogFile, wantLogSuffix)
	}

	dataDir := config.ServiceDataDir("s3", "latest")
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
