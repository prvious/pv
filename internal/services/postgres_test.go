package services

import "testing"

func TestPostgresPort(t *testing.T) {
	p := &Postgres{}
	tests := []struct {
		version string
		want    int
	}{
		{"latest", 54000},
		{"16", 54016},
		{"17", 54017},
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
	if opts.HealthCmd[1] != "pg_isready" {
		t.Errorf("HealthCmd = %v, want pg_isready", opts.HealthCmd)
	}
}
