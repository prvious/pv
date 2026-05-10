package projectenv

import (
	"strings"
	"testing"

	"github.com/prvious/pv/internal/certs"
)

func TestProjectTemplateVars_Defaults(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	got := ProjectTemplateVars("myapp", "test")

	if got["site_url"] != "https://myapp.test" {
		t.Errorf("site_url = %q, want %q", got["site_url"], "https://myapp.test")
	}
	if got["site_host"] != "myapp.test" {
		t.Errorf("site_host = %q, want %q", got["site_host"], "myapp.test")
	}
	if got["tls_cert_path"] != certs.CertPath("myapp.test") {
		t.Errorf("tls_cert_path = %q, want %q", got["tls_cert_path"], certs.CertPath("myapp.test"))
	}
	if got["tls_key_path"] != certs.KeyPath("myapp.test") {
		t.Errorf("tls_key_path = %q, want %q", got["tls_key_path"], certs.KeyPath("myapp.test"))
	}
	if !strings.HasSuffix(got["tls_cert_path"], "myapp.test.crt") {
		t.Errorf("tls_cert_path should end with myapp.test.crt, got %q", got["tls_cert_path"])
	}
}

func TestProjectTemplateVars_CustomTLD(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	got := ProjectTemplateVars("myapp", "dev")

	if got["site_url"] != "https://myapp.dev" {
		t.Errorf("site_url = %q, want %q", got["site_url"], "https://myapp.dev")
	}
	if got["site_host"] != "myapp.dev" {
		t.Errorf("site_host = %q, want %q", got["site_host"], "myapp.dev")
	}
}
