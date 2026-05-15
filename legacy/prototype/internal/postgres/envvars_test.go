package postgres

import "testing"

func TestEnvVars_Golden(t *testing.T) {
	got, err := EnvVars("my_app", "17")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
	want := map[string]string{
		"DB_CONNECTION": "pgsql",
		"DB_HOST":       "127.0.0.1",
		"DB_PORT":       "54017",
		"DB_DATABASE":   "my_app",
		"DB_USERNAME":   "postgres",
		"DB_PASSWORD":   "postgres",
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("%s = %q, want %q", k, got[k], v)
		}
	}
}

func TestEnvVars_Pg18Port(t *testing.T) {
	got, err := EnvVars("my_app", "18")
	if err != nil {
		t.Fatalf("EnvVars: %v", err)
	}
	if got["DB_PORT"] != "54018" {
		t.Errorf("DB_PORT = %q, want 54018", got["DB_PORT"])
	}
}
