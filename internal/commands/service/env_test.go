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

func TestEnvVarsFor_DockerService(t *testing.T) {
	// Redis is the only remaining docker service. Its EnvVars ignores the
	// port arg (Redis.Port() returns the fixed 6379), but the test still
	// exercises the docker branch of envVarsFor's switch.
	got, err := envVarsFor("redis", "anyproject", 6379)
	if err != nil {
		t.Fatalf("envVarsFor(\"redis\") error = %v", err)
	}
	if got["REDIS_HOST"] != "127.0.0.1" {
		t.Errorf("REDIS_HOST = %q, want 127.0.0.1", got["REDIS_HOST"])
	}
}

func TestEnvVarsFor_Unknown(t *testing.T) {
	_, err := envVarsFor("mongodb", "anyproject", 0)
	if err == nil {
		t.Fatal("expected error for unknown service")
	}
	if !strings.Contains(err.Error(), `unknown service "mongodb"`) {
		t.Errorf("error %q missing expected text", err)
	}
}
