package mysql

import "testing"

func TestEnvVars_Golden84(t *testing.T) {
	got, err := EnvVars("my_app", "8.4")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
	want := map[string]string{
		"DB_CONNECTION": "mysql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       "33084",
		"DB_DATABASE":   "my_app",
		"DB_USERNAME":   "root",
		"DB_PASSWORD":   "",
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("%s = %q, want %q", k, got[k], v)
		}
	}
}

func TestEnvVars_Mysql80Port(t *testing.T) {
	got, err := EnvVars("my_app", "8.0")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
	if got["DB_PORT"] != "33080" {
		t.Errorf("DB_PORT = %q, want 33080", got["DB_PORT"])
	}
}

func TestEnvVars_Mysql97Port(t *testing.T) {
	got, err := EnvVars("my_app", "9.7")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
	if got["DB_PORT"] != "33097" {
		t.Errorf("DB_PORT = %q, want 33097", got["DB_PORT"])
	}
}

func TestEnvVars_InvalidVersion_Errors(t *testing.T) {
	if _, err := EnvVars("my_app", "garbage"); err == nil {
		t.Error("expected error on invalid version")
	}
}
