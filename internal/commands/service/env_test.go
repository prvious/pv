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
	// MySQL.EnvVars takes (projectName, port) and uses both. Pass a non-default
	// port to verify the port arg is consulted — a regression that always
	// passed 0 in the docker branch would silently set DB_PORT=0.
	got, err := envVarsFor("mysql", "anyproject", 3306)
	if err != nil {
		t.Fatalf("envVarsFor(\"mysql\") error = %v", err)
	}
	if got["DB_PORT"] != "3306" {
		t.Errorf("DB_PORT = %q, want 3306", got["DB_PORT"])
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
