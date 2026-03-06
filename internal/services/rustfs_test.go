package services

import "testing"

func TestRustFSPorts(t *testing.T) {
	r := &RustFS{}
	if got := r.Port("latest"); got != 9000 {
		t.Errorf("Port = %d, want 9000", got)
	}
	if got := r.ConsolePort("latest"); got != 9001 {
		t.Errorf("ConsolePort = %d, want 9001", got)
	}
}

func TestRustFSImageName(t *testing.T) {
	r := &RustFS{}
	if got := r.ImageName("latest"); got != "rustfs/rustfs:latest" {
		t.Errorf("ImageName = %q, want %q", got, "rustfs/rustfs:latest")
	}
}

func TestRustFSEnvVars(t *testing.T) {
	r := &RustFS{}
	env := r.EnvVars("my_app", 9000)
	if env["AWS_ACCESS_KEY_ID"] != "minioadmin" {
		t.Errorf("AWS_ACCESS_KEY_ID = %q", env["AWS_ACCESS_KEY_ID"])
	}
	if env["AWS_BUCKET"] != "my_app" {
		t.Errorf("AWS_BUCKET = %q", env["AWS_BUCKET"])
	}
	if env["AWS_ENDPOINT"] != "http://127.0.0.1:9000" {
		t.Errorf("AWS_ENDPOINT = %q", env["AWS_ENDPOINT"])
	}
	if env["AWS_USE_PATH_STYLE_ENDPOINT"] != "true" {
		t.Errorf("AWS_USE_PATH_STYLE_ENDPOINT = %q", env["AWS_USE_PATH_STYLE_ENDPOINT"])
	}
}

func TestRustFSCreateOpts(t *testing.T) {
	r := &RustFS{}
	opts := r.CreateOpts("latest")
	if len(opts.Cmd) == 0 {
		t.Error("expected Cmd to be set for RustFS")
	}
	if opts.Ports[9001] != 9001 {
		t.Error("expected console port 9001 mapping")
	}
}
