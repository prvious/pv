package rustfs

import (
	"testing"
)

func TestTemplateVars(t *testing.T) {
	got := TemplateVars()

	if got["endpoint"] != "http://127.0.0.1:9000" {
		t.Errorf("endpoint = %q, want http://127.0.0.1:9000", got["endpoint"])
	}
	if got["access_key"] != "rstfsadmin" {
		t.Errorf("access_key = %q, want rstfsadmin", got["access_key"])
	}
	if got["secret_key"] != "rstfsadmin" {
		t.Errorf("secret_key = %q, want rstfsadmin", got["secret_key"])
	}
	if got["region"] != "us-east-1" {
		t.Errorf("region = %q, want us-east-1", got["region"])
	}
	if got["use_path_style"] != "true" {
		t.Errorf("use_path_style = %q, want true", got["use_path_style"])
	}
}
