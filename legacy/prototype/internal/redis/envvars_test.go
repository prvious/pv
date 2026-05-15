package redis

import "testing"

func TestEnvVars_ContainsExpectedKeys(t *testing.T) {
	vars := EnvVars("8.6", "myapp")
	if vars["REDIS_HOST"] != "127.0.0.1" {
		t.Errorf("REDIS_HOST = %q", vars["REDIS_HOST"])
	}
	if vars["REDIS_PORT"] != "7160" {
		t.Errorf("REDIS_PORT = %q", vars["REDIS_PORT"])
	}
	if vars["REDIS_PASSWORD"] != "null" {
		t.Errorf("REDIS_PASSWORD = %q", vars["REDIS_PASSWORD"])
	}
}
