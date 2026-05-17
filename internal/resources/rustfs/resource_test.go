package rustfs

import (
	"testing"

	"github.com/prvious/pv/internal/control"
)

func TestRustFSRedactsCredentialsFromStatus(t *testing.T) {
	desired := Desired("1.0.0")
	if desired.Resource != control.ResourceRustFS {
		t.Fatalf("resource = %q", desired.Resource)
	}
	credentials := Credentials{AccessKey: "local", SecretKey: "secret"}
	env := Env("1.0.0", credentials)
	if env["AWS_SECRET_ACCESS_KEY"] != "secret" {
		t.Fatalf("env should retain secret for rendering: %#v", env)
	}
	if env["AWS_ENDPOINT"] != "http://127.0.0.1:9000" {
		t.Fatalf("AWS_ENDPOINT = %q, want http://127.0.0.1:9000", env["AWS_ENDPOINT"])
	}
	if env["AWS_USE_PATH_STYLE_ENDPOINT"] != "true" {
		t.Fatalf("AWS_USE_PATH_STYLE_ENDPOINT = %q, want true", env["AWS_USE_PATH_STYLE_ENDPOINT"])
	}
	for _, key := range []string{"AWS_ENDPOINT_URL", "AWS_URL"} {
		if _, ok := env[key]; ok {
			t.Fatalf("env should not include %s before bucket is known: %#v", key, env)
		}
	}
	status := RedactedStatus(credentials)
	if status["AWS_ACCESS_KEY_ID"] != "<redacted>" || status["AWS_SECRET_ACCESS_KEY"] != "<redacted>" {
		t.Fatalf("status did not redact credentials: %#v", status)
	}
	if status["AWS_ENDPOINT"] != "http://127.0.0.1:9000" {
		t.Fatalf("status AWS_ENDPOINT = %q, want http://127.0.0.1:9000", status["AWS_ENDPOINT"])
	}
	if status["AWS_USE_PATH_STYLE_ENDPOINT"] != "true" {
		t.Fatalf("status AWS_USE_PATH_STYLE_ENDPOINT = %q, want true", status["AWS_USE_PATH_STYLE_ENDPOINT"])
	}
	if _, ok := status["AWS_ENDPOINT_URL"]; ok {
		t.Fatalf("status should not include AWS_ENDPOINT_URL: %#v", status)
	}
}
