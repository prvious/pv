package mysql

import (
	"testing"

	"github.com/prvious/pv/internal/control"
)

func TestMySQLResourceIsExplicit(t *testing.T) {
	desired := Desired("9.7")
	if desired.Resource != control.ResourceMySQL {
		t.Fatalf("resource = %q", desired.Resource)
	}
	env := Env("9.7")
	if env["DB_CONNECTION"] != "mysql" || env["PV_MYSQL"] != "9.7" {
		t.Fatalf("env = %#v", env)
	}
}
