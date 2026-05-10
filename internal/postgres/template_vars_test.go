package postgres

import (
	"testing"
)

func TestTemplateVars_Major18(t *testing.T) {
	got, err := TemplateVars("18", "18.1")
	if err != nil {
		t.Fatalf("TemplateVars() error = %v", err)
	}
	if got["host"] != "127.0.0.1" {
		t.Errorf("host = %q, want 127.0.0.1", got["host"])
	}
	if got["port"] != "54018" {
		t.Errorf("port = %q, want 54018", got["port"])
	}
	if got["username"] != "postgres" {
		t.Errorf("username = %q, want postgres", got["username"])
	}
	if got["password"] != "postgres" {
		t.Errorf("password = %q, want postgres", got["password"])
	}
	if got["version"] != "18.1" {
		t.Errorf("version = %q, want 18.1", got["version"])
	}
	if got["dsn"] != "postgresql://postgres:postgres@127.0.0.1:54018" {
		t.Errorf("dsn = %q, want postgresql://postgres:postgres@127.0.0.1:54018", got["dsn"])
	}
}

func TestTemplateVars_InvalidMajorPropagates(t *testing.T) {
	_, err := TemplateVars("not-a-number", "")
	if err == nil {
		t.Fatal("TemplateVars() with bad major: want error, got nil")
	}
}
