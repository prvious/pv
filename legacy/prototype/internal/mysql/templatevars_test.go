package mysql

import (
	"testing"
)

func TestTemplateVars_Version80(t *testing.T) {
	got, err := TemplateVars("8.0", "8.0.36")
	if err != nil {
		t.Fatalf("TemplateVars() error = %v", err)
	}
	if got["host"] != "127.0.0.1" {
		t.Errorf("host = %q, want 127.0.0.1", got["host"])
	}
	if got["port"] != "33080" {
		t.Errorf("port = %q, want 33080", got["port"])
	}
	if got["username"] != "root" {
		t.Errorf("username = %q, want root", got["username"])
	}
	if got["password"] != "" {
		t.Errorf("password = %q, want empty", got["password"])
	}
	if got["version"] != "8.0.36" {
		t.Errorf("version = %q, want 8.0.36", got["version"])
	}
	if got["dsn"] != "mysql://root:@127.0.0.1:33080" {
		t.Errorf("dsn = %q, want mysql://root:@127.0.0.1:33080", got["dsn"])
	}
}

func TestTemplateVars_InvalidVersionPropagates(t *testing.T) {
	_, err := TemplateVars("not-a-version", "")
	if err == nil {
		t.Fatal("TemplateVars() with bad version: want error, got nil")
	}
}
