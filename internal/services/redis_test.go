package services

import "testing"

func TestRedisPort(t *testing.T) {
	r := &Redis{}
	if got := r.Port("latest"); got != 6379 {
		t.Errorf("Redis.Port() = %d, want 6379", got)
	}
	if got := r.Port("7.2"); got != 6379 {
		t.Errorf("Redis.Port(7.2) = %d, want 6379", got)
	}
}

func TestRedisEnvVars(t *testing.T) {
	r := &Redis{}
	env := r.EnvVars("", 6379)
	if env["REDIS_HOST"] != "127.0.0.1" {
		t.Errorf("REDIS_HOST = %q", env["REDIS_HOST"])
	}
	if env["REDIS_PORT"] != "6379" {
		t.Errorf("REDIS_PORT = %q", env["REDIS_PORT"])
	}
	if env["REDIS_PASSWORD"] != "null" {
		t.Errorf("REDIS_PASSWORD = %q", env["REDIS_PASSWORD"])
	}
}

func TestRedisHasNoDatabases(t *testing.T) {
	r := &Redis{}
	if r.HasDatabases() {
		t.Error("Redis.HasDatabases() should return false")
	}
}

func TestRedisCreateOpts(t *testing.T) {
	r := &Redis{}
	opts := r.CreateOpts("latest")
	if opts.HealthCmd[1] != "redis-cli ping" {
		t.Errorf("HealthCmd = %v, want redis-cli ping", opts.HealthCmd)
	}
}
