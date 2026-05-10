package service

import (
	"strings"
	"testing"
)

func TestEnvVarsFor_BinaryService(t *testing.T) {
	got, err := envVarsFor("mail", "anyproject", 0)
	if err != nil {
		t.Fatalf("envVarsFor(\"mail\") error = %v", err)
	}
	if got["MAIL_MAILER"] != "smtp" {
		t.Errorf("MAIL_MAILER = %q, want smtp", got["MAIL_MAILER"])
	}
	if got["MAIL_HOST"] != "127.0.0.1" {
		t.Errorf("MAIL_HOST = %q, want 127.0.0.1", got["MAIL_HOST"])
	}
}

// Note: TestEnvVarsFor_DockerService was dropped when redis (the last
// docker Service) migrated to a native binary. The docker registry is
// now empty; the docker branch of envVarsFor's switch is unreachable
// from real callers. Restore this test against any future docker
// service if one is reintroduced.

func TestEnvVarsFor_Unknown(t *testing.T) {
	_, err := envVarsFor("mongodb", "anyproject", 0)
	if err == nil {
		t.Fatal("expected error for unknown service")
	}
	if !strings.Contains(err.Error(), `unknown service "mongodb"`) {
		t.Errorf("error %q missing expected text", err)
	}
}
