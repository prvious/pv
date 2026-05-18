package redis

import "testing"

func TestTemplateVars(t *testing.T) {
	vars := TemplateVars("8.6")
	if vars["host"] != "127.0.0.1" {
		t.Errorf("host = %q", vars["host"])
	}
	if vars["port"] != "7160" {
		t.Errorf("port = %q", vars["port"])
	}
	if vars["password"] != "" {
		t.Errorf("password = %q", vars["password"])
	}
	if vars["url"] != "redis://127.0.0.1:7160" {
		t.Errorf("url = %q", vars["url"])
	}
}
