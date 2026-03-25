package services

import "testing"

func TestPostgresDefaultVersion(t *testing.T) {
	p := &Postgres{}
	if got := p.DefaultVersion(); got != "18-alpine" {
		t.Errorf("DefaultVersion = %q, want %q", got, "18-alpine")
	}
}

func TestPostgresPort(t *testing.T) {
	p := &Postgres{}
	tests := []struct {
		version string
		want    int
	}{
		{"latest", 54000},
		{"16", 54016},
		{"17", 54017},
		{"18-alpine", 54018},
		{"16-bullseye", 54016},
		{"alpine", 54000}, // purely non-numeric version falls back to base port
	}
	for _, tt := range tests {
		got := p.Port(tt.version)
		if got != tt.want {
			t.Errorf("Postgres.Port(%q) = %d, want %d", tt.version, got, tt.want)
		}
	}
}

func TestPostgresImageName(t *testing.T) {
	p := &Postgres{}
	if got := p.ImageName("16"); got != "postgres:16" {
		t.Errorf("ImageName = %q, want %q", got, "postgres:16")
	}
}

func TestPostgresContainerName(t *testing.T) {
	p := &Postgres{}
	if got := p.ContainerName("16"); got != "pv-postgres-16" {
		t.Errorf("ContainerName = %q, want %q", got, "pv-postgres-16")
	}
}

func TestPostgresEnvVars(t *testing.T) {
	p := &Postgres{}
	env := p.EnvVars("my_app", 54016)
	if env["DB_CONNECTION"] != "pgsql" {
		t.Errorf("DB_CONNECTION = %q, want %q", env["DB_CONNECTION"], "pgsql")
	}
	if env["DB_USERNAME"] != "postgres" {
		t.Errorf("DB_USERNAME = %q, want %q", env["DB_USERNAME"], "postgres")
	}
}

func TestPostgresCreateOpts(t *testing.T) {
	p := &Postgres{}
	opts := p.CreateOpts("16")

	if opts.HealthCmd[1] != "pg_isready -d postgres -U postgres" {
		t.Errorf("HealthCmd = %v, want pg_isready -d postgres -U postgres", opts.HealthCmd)
	}
	if opts.HealthRetries != 20 {
		t.Errorf("HealthRetries = %d, want 20", opts.HealthRetries)
	}
	if opts.HealthInterval != "3s" {
		t.Errorf("HealthInterval = %q, want %q", opts.HealthInterval, "3s")
	}

	// Verify env vars include user and password.
	hasUser, hasPassword := false, false
	for _, env := range opts.Env {
		if env == "POSTGRES_USER=postgres" {
			hasUser = true
		}
		if env == "POSTGRES_PASSWORD=postgres" {
			hasPassword = true
		}
	}
	if !hasUser {
		t.Error("expected POSTGRES_USER=postgres in env")
	}
	if !hasPassword {
		t.Error("expected POSTGRES_PASSWORD=postgres in env")
	}

	// Verify volume mount target.
	found := false
	for _, target := range opts.Volumes {
		if target == "/var/lib/postgresql" {
			found = true
		}
	}
	if !found {
		t.Error("expected volume mount target /var/lib/postgresql")
	}
}

func TestPostgresEnvVars_Password(t *testing.T) {
	p := &Postgres{}
	env := p.EnvVars("my_app", 54016)
	if env["DB_PASSWORD"] != "postgres" {
		t.Errorf("DB_PASSWORD = %q, want %q", env["DB_PASSWORD"], "postgres")
	}
}
