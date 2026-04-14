package services

import (
	"reflect"
	"testing"
)

func TestRustFS_RegisteredAsS3(t *testing.T) {
	svc, ok := LookupBinary("s3")
	if !ok {
		t.Fatal("LookupBinary(\"s3\") returned ok=false")
	}
	if _, isRustfs := svc.(*RustFS); !isRustfs {
		t.Errorf("expected *RustFS, got %T", svc)
	}
}

func TestRustFS_Name(t *testing.T) {
	r := &RustFS{}
	if r.Name() != "s3" {
		t.Errorf("Name() = %q, want s3", r.Name())
	}
}

func TestRustFS_Ports(t *testing.T) {
	r := &RustFS{}
	if r.Port() != 9000 {
		t.Errorf("Port() = %d, want 9000", r.Port())
	}
	if r.ConsolePort() != 9001 {
		t.Errorf("ConsolePort() = %d, want 9001", r.ConsolePort())
	}
}

func TestRustFS_WebRoutes(t *testing.T) {
	r := &RustFS{}
	want := []WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
	got := r.WebRoutes()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("WebRoutes() = %#v, want %#v", got, want)
	}
}

func TestRustFS_EnvVars_MatchesDockerKeys(t *testing.T) {
	// Linked projects rely on these exact .env keys; the binary migration
	// must not silently change them.
	r := &RustFS{}
	vars := r.EnvVars("myproject")
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

func TestRustFS_Args_IncludesDataDir(t *testing.T) {
	r := &RustFS{}
	args := r.Args("/tmp/data")
	found := false
	for _, a := range args {
		if a == "/tmp/data" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("Args() did not include the data dir; got %v", args)
	}
}

func TestRustFS_Args_IncludesConsoleEnable(t *testing.T) {
	// Verified 2026-04-14: port 9001 does not bind unless --console-enable
	// is passed explicitly.
	r := &RustFS{}
	args := r.Args("/tmp/data")
	found := false
	for _, a := range args {
		if a == "--console-enable" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("Args() missing --console-enable; got %v", args)
	}
}

func TestRustFS_Env_UsesAccessSecretKeys(t *testing.T) {
	r := &RustFS{}
	env := r.Env()
	var sawAccess, sawSecret bool
	for _, e := range env {
		if e == "RUSTFS_ACCESS_KEY=rstfsadmin" {
			sawAccess = true
		}
		if e == "RUSTFS_SECRET_KEY=rstfsadmin" {
			sawSecret = true
		}
	}
	if !sawAccess {
		t.Errorf("Env() missing RUSTFS_ACCESS_KEY; got %v", env)
	}
	if !sawSecret {
		t.Errorf("Env() missing RUSTFS_SECRET_KEY; got %v", env)
	}
}

func TestRustFS_ReadyCheck_TCP9000(t *testing.T) {
	r := &RustFS{}
	rc := r.ReadyCheck()
	if rc.TCPPort != 9000 {
		t.Errorf("ReadyCheck.TCPPort = %d, want 9000", rc.TCPPort)
	}
	if rc.HTTPEndpoint != "" {
		t.Errorf("ReadyCheck.HTTPEndpoint = %q, want empty (TCP probe)", rc.HTTPEndpoint)
	}
	if rc.Timeout == 0 {
		t.Error("ReadyCheck.Timeout must be non-zero")
	}
}
