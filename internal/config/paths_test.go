package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPvDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PvDir()
	if got != filepath.Join(home, ".pv") {
		t.Errorf("PvDir() = %q, want %q", got, filepath.Join(home, ".pv"))
	}
}

func TestConfigDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ConfigDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "config")) {
		t.Errorf("ConfigDir() = %q, want suffix .pv/config", got)
	}
}

func TestSitesDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := SitesDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "config", "sites")) {
		t.Errorf("SitesDir() = %q, want suffix .pv/config/sites", got)
	}
}

func TestLogsDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := LogsDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "logs")) {
		t.Errorf("LogsDir() = %q, want suffix .pv/logs", got)
	}
}

func TestDataDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := DataDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "data")) {
		t.Errorf("DataDir() = %q, want suffix .pv/data", got)
	}
}

func TestBinDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := BinDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "bin")) {
		t.Errorf("BinDir() = %q, want suffix .pv/bin", got)
	}
}

func TestRegistryPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := RegistryPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "data", "registry.json")) {
		t.Errorf("RegistryPath() = %q, want suffix .pv/data/registry.json", got)
	}
}

func TestVersionsPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := VersionsPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "data", "versions.json")) {
		t.Errorf("VersionsPath() = %q, want suffix .pv/data/versions.json", got)
	}
}

func TestSettingsPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := SettingsPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "config", "settings.json")) {
		t.Errorf("SettingsPath() = %q, want suffix .pv/config/settings.json", got)
	}
}

func TestCaddyfilePath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := CaddyfilePath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "config", "Caddyfile")) {
		t.Errorf("CaddyfilePath() = %q, want suffix .pv/config/Caddyfile", got)
	}
}

func TestPidFilePath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PidFilePath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "data", "pv.pid")) {
		t.Errorf("PidFilePath() = %q, want suffix .pv/data/pv.pid", got)
	}
}

func TestCaddyLogPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := CaddyLogPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "logs", "caddy.log")) {
		t.Errorf("CaddyLogPath() = %q, want suffix .pv/logs/caddy.log", got)
	}
}

func TestDNSPort(t *testing.T) {
	if DNSPort != 10053 {
		t.Errorf("DNSPort = %d, want 10053", DNSPort)
	}
}

func TestPhpDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "php")) {
		t.Errorf("PhpDir() = %q, want suffix .pv/php", got)
	}
}

func TestPhpVersionDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpVersionDir("8.4")
	if !strings.HasSuffix(got, filepath.Join(".pv", "php", "8.4")) {
		t.Errorf("PhpVersionDir(8.4) = %q, want suffix .pv/php/8.4", got)
	}
}

func TestVersionSitesDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := VersionSitesDir("8.3")
	if !strings.HasSuffix(got, filepath.Join(".pv", "config", "sites-8.3")) {
		t.Errorf("VersionSitesDir(8.3) = %q, want suffix .pv/config/sites-8.3", got)
	}
}

func TestVersionCaddyfilePath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := VersionCaddyfilePath("8.3")
	if !strings.HasSuffix(got, filepath.Join(".pv", "config", "php-8.3.Caddyfile")) {
		t.Errorf("VersionCaddyfilePath(8.3) = %q, want suffix .pv/config/php-8.3.Caddyfile", got)
	}
}

func TestPortForVersion(t *testing.T) {
	tests := []struct {
		version string
		want    int
	}{
		{"8.3", 8830},
		{"8.4", 8840},
		{"8.5", 8850},
		{"9.0", 8900},
	}
	for _, tt := range tests {
		got := PortForVersion(tt.version)
		if got != tt.want {
			t.Errorf("PortForVersion(%q) = %d, want %d", tt.version, got, tt.want)
		}
	}
}

func TestCaddyEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	env := CaddyEnv()
	if len(env) != 2 {
		t.Fatalf("CaddyEnv() returned %d entries, want 2", len(env))
	}
	pvDir := filepath.Join(home, ".pv")
	if env[0] != "XDG_DATA_HOME="+pvDir {
		t.Errorf("CaddyEnv()[0] = %q, want %q", env[0], "XDG_DATA_HOME="+pvDir)
	}
	if env[1] != "XDG_CONFIG_HOME="+pvDir {
		t.Errorf("CaddyEnv()[1] = %q, want %q", env[1], "XDG_CONFIG_HOME="+pvDir)
	}
}

func TestCACertPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := CACertPath()
	want := filepath.Join(home, ".pv", "caddy", "pki", "authorities", "local", "root.crt")
	if got != want {
		t.Errorf("CACertPath() = %q, want %q", got, want)
	}
}

func TestEnsureDirs(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	dirs := []string{
		ConfigDir(),
		SitesDir(),
		LogsDir(),
		DataDir(),
		BinDir(),
		PhpDir(),
	}
	for _, dir := range dirs {
		info, err := os.Stat(dir)
		if err != nil {
			t.Errorf("directory %q does not exist after EnsureDirs()", dir)
			continue
		}
		if !info.IsDir() {
			t.Errorf("%q is not a directory", dir)
		}
	}
}

func TestEnsureDirs_Idempotent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatalf("first EnsureDirs() error = %v", err)
	}
	if err := EnsureDirs(); err != nil {
		t.Fatalf("second EnsureDirs() error = %v", err)
	}
}
