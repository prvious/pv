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
	status := RedactedStatus(credentials)
	if status["AWS_ACCESS_KEY_ID"] != "<redacted>" || status["AWS_SECRET_ACCESS_KEY"] != "<redacted>" {
		t.Fatalf("status did not redact credentials: %#v", status)
	}
}
