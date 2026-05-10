package redis

import (
	"testing"
)

func TestTemplateVars(t *testing.T) {
	got := TemplateVars()

	if got["host"] != "127.0.0.1" {
		t.Errorf("host = %q, want 127.0.0.1", got["host"])
	}
	if got["port"] != "6379" {
		t.Errorf("port = %q, want 6379", got["port"])
	}
	if got["password"] != "" {
		t.Errorf("password = %q, want empty", got["password"])
	}
	if got["url"] != "redis://127.0.0.1:6379" {
		t.Errorf("url = %q, want redis://127.0.0.1:6379", got["url"])
	}
}
