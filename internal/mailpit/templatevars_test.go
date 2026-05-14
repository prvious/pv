package mailpit

import (
	"testing"
)

func TestTemplateVars(t *testing.T) {
	got := TemplateVars(DefaultVersion())

	if got["smtp_host"] != "127.0.0.1" {
		t.Errorf("smtp_host = %q, want 127.0.0.1", got["smtp_host"])
	}
	if got["smtp_port"] != "1025" {
		t.Errorf("smtp_port = %q, want 1025", got["smtp_port"])
	}
	if got["http_host"] != "127.0.0.1" {
		t.Errorf("http_host = %q, want 127.0.0.1", got["http_host"])
	}
	if got["http_port"] != "8025" {
		t.Errorf("http_port = %q, want 8025", got["http_port"])
	}
}

func TestTemplateVars_RejectsLatest(t *testing.T) {
	got := TemplateVars("latest")

	if len(got) != 0 {
		t.Errorf("TemplateVars(latest) = %#v, want empty map", got)
	}
}
