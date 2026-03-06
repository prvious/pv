package services

import "testing"

func TestMySQLPort(t *testing.T) {
	m := &MySQL{}
	tests := []struct {
		version string
		want    int
	}{
		{"latest", 33000},
		{"8.0.32", 33032},
		{"8.0.45", 33045},
		{"8.0", 33000},
		{"8", 33000},
	}
	for _, tt := range tests {
		got := m.Port(tt.version)
		if got != tt.want {
			t.Errorf("MySQL.Port(%q) = %d, want %d", tt.version, got, tt.want)
		}
	}
}

func TestMySQLImageName(t *testing.T) {
	m := &MySQL{}
	if got := m.ImageName("8.0.32"); got != "mysql:8.0.32" {
		t.Errorf("ImageName = %q, want %q", got, "mysql:8.0.32")
	}
	if got := m.ImageName("latest"); got != "mysql:latest" {
		t.Errorf("ImageName = %q, want %q", got, "mysql:latest")
	}
}

func TestMySQLContainerName(t *testing.T) {
	m := &MySQL{}
	if got := m.ContainerName("8.0.32"); got != "pv-mysql-8.0.32" {
		t.Errorf("ContainerName = %q, want %q", got, "pv-mysql-8.0.32")
	}
}

func TestMySQLDefaultVersion(t *testing.T) {
	m := &MySQL{}
	if got := m.DefaultVersion(); got != "latest" {
		t.Errorf("DefaultVersion = %q, want %q", got, "latest")
	}
}

func TestMySQLEnvVars(t *testing.T) {
	m := &MySQL{}
	env := m.EnvVars("my_app", 33032)
	if env["DB_CONNECTION"] != "mysql" {
		t.Errorf("DB_CONNECTION = %q, want %q", env["DB_CONNECTION"], "mysql")
	}
	if env["DB_PORT"] != "33032" {
		t.Errorf("DB_PORT = %q, want %q", env["DB_PORT"], "33032")
	}
	if env["DB_DATABASE"] != "my_app" {
		t.Errorf("DB_DATABASE = %q, want %q", env["DB_DATABASE"], "my_app")
	}
	if env["DB_USERNAME"] != "root" {
		t.Errorf("DB_USERNAME = %q, want %q", env["DB_USERNAME"], "root")
	}
}

func TestMySQLCreateOpts(t *testing.T) {
	m := &MySQL{}
	opts := m.CreateOpts("8.0.32")
	if opts.Name != "pv-mysql-8.0.32" {
		t.Errorf("Name = %q, want %q", opts.Name, "pv-mysql-8.0.32")
	}
	if opts.Image != "mysql:8.0.32" {
		t.Errorf("Image = %q, want %q", opts.Image, "mysql:8.0.32")
	}
	if len(opts.HealthCmd) == 0 {
		t.Error("expected HealthCmd to be set")
	}
	if opts.Labels["dev.prvious.pv"] != "true" {
		t.Error("expected pv label")
	}
}

func TestMySQLHasDatabases(t *testing.T) {
	m := &MySQL{}
	if !m.HasDatabases() {
		t.Error("MySQL.HasDatabases() should return true")
	}
}
