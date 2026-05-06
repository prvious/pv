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
	if !strings.HasSuffix(got, filepath.Join(".pv", "pv.yml")) {
		t.Errorf("SettingsPath() = %q, want suffix .pv/pv.yml", got)
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

func TestCaddyStderrPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := CaddyStderrPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "logs", "caddy-stderr.log")) {
		t.Errorf("CaddyStderrPath() = %q, want suffix .pv/logs/caddy-stderr.log", got)
	}
}

func TestCaddyStderrPathForVersion(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := CaddyStderrPathForVersion("8.3")
	if !strings.HasSuffix(got, filepath.Join(".pv", "logs", "caddy-8.3-stderr.log")) {
		t.Errorf("CaddyStderrPathForVersion(8.3) = %q, want suffix .pv/logs/caddy-8.3-stderr.log", got)
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
	if len(env) != 4 {
		t.Fatalf("CaddyEnv() returned %d entries, want 4", len(env))
	}
	pvDir := filepath.Join(home, ".pv")
	if env[0] != "XDG_DATA_HOME="+pvDir {
		t.Errorf("CaddyEnv()[0] = %q, want %q", env[0], "XDG_DATA_HOME="+pvDir)
	}
	if env[1] != "XDG_CONFIG_HOME="+pvDir {
		t.Errorf("CaddyEnv()[1] = %q, want %q", env[1], "XDG_CONFIG_HOME="+pvDir)
	}
	if env[2] != "COMPOSER_HOME="+filepath.Join(pvDir, "composer") {
		t.Errorf("CaddyEnv()[2] = %q, want COMPOSER_HOME", env[2])
	}
	if env[3] != "COMPOSER_CACHE_DIR="+filepath.Join(pvDir, "composer", "cache") {
		t.Errorf("CaddyEnv()[3] = %q, want COMPOSER_CACHE_DIR", env[3])
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

func TestComposerDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ComposerDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "composer")) {
		t.Errorf("ComposerDir() = %q, want suffix .pv/composer", got)
	}
}

func TestComposerCacheDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ComposerCacheDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "composer", "cache")) {
		t.Errorf("ComposerCacheDir() = %q, want suffix .pv/composer/cache", got)
	}
}

func TestComposerBinDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ComposerBinDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "composer", "vendor", "bin")) {
		t.Errorf("ComposerBinDir() = %q, want suffix .pv/composer/vendor/bin", got)
	}
}

func TestComposerPharPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ComposerPharPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "internal", "bin", "composer.phar")) {
		t.Errorf("ComposerPharPath() = %q, want suffix .pv/internal/bin/composer.phar", got)
	}
}

func TestMagoPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := MagoPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "internal", "bin", "mago")) {
		t.Errorf("MagoPath() = %q, want suffix .pv/internal/bin/mago", got)
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
		ComposerDir(),
		ComposerCacheDir(),
		InternalBinDir(),
		PackagesDir(),
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

func TestInternalBinDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := InternalBinDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "internal", "bin")) {
		t.Errorf("InternalBinDir() = %q, want suffix .pv/internal/bin", got)
	}
}

func TestColimaPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ColimaPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "internal", "bin", "colima")) {
		t.Errorf("ColimaPath() = %q, want suffix .pv/internal/bin/colima", got)
	}
}

func TestColimaSocketPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ColimaSocketPath()
	if !strings.HasSuffix(got, filepath.Join(".pv", "internal", "colima", "pv", "docker.sock")) {
		t.Errorf("ColimaSocketPath() = %q, want suffix .pv/internal/colima/pv/docker.sock", got)
	}
}

func TestColimaHomeDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := ColimaHomeDir()
	if !strings.HasSuffix(got, filepath.Join(".pv", "internal", "colima")) {
		t.Errorf("ColimaHomeDir() = %q, want suffix .pv/internal/colima", got)
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

func TestPackagesDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PackagesDir()
	want := filepath.Join(home, ".pv", "internal", "packages")
	if got != want {
		t.Errorf("PackagesDir() = %q, want %q", got, want)
	}
}

func TestEnsureDirs_CreatesPackagesDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := EnsureDirs(); err != nil {
		t.Fatalf("EnsureDirs() error = %v", err)
	}

	if _, err := os.Stat(PackagesDir()); os.IsNotExist(err) {
		t.Error("EnsureDirs() did not create PackagesDir()")
	}
}

func TestPhpEtcDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpEtcDir("8.4")
	want := filepath.Join(home, ".pv", "php", "8.4", "etc")
	if got != want {
		t.Errorf("PhpEtcDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpConfDDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpConfDDir("8.4")
	want := filepath.Join(home, ".pv", "php", "8.4", "conf.d")
	if got != want {
		t.Errorf("PhpConfDDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpSessionDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpSessionDir("8.4")
	want := filepath.Join(home, ".pv", "data", "sessions", "8.4")
	if got != want {
		t.Errorf("PhpSessionDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpTmpDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpTmpDir("8.4")
	want := filepath.Join(home, ".pv", "data", "tmp", "8.4")
	if got != want {
		t.Errorf("PhpTmpDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpEnv("8.4")
	wantPHPRC := "PHPRC=" + filepath.Join(home, ".pv", "php", "8.4", "etc")
	wantScan := "PHP_INI_SCAN_DIR=" + filepath.Join(home, ".pv", "php", "8.4", "conf.d")

	if len(got) != 2 {
		t.Fatalf("PhpEnv() returned %d entries, want 2", len(got))
	}
	if got[0] != wantPHPRC {
		t.Errorf("PhpEnv()[0] = %q, want %q", got[0], wantPHPRC)
	}
	if got[1] != wantScan {
		t.Errorf("PhpEnv()[1] = %q, want %q", got[1], wantScan)
	}
}

func TestPostgresDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresDir()
	want := "/home/test/.pv/postgres"
	if got != want {
		t.Errorf("PostgresDir = %q, want %q", got, want)
	}
}

func TestPostgresVersionDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresVersionDir("17")
	want := "/home/test/.pv/postgres/17"
	if got != want {
		t.Errorf("PostgresVersionDir = %q, want %q", got, want)
	}
}

func TestPostgresBinDir(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresBinDir("17")
	want := "/home/test/.pv/postgres/17/bin"
	if got != want {
		t.Errorf("PostgresBinDir = %q, want %q", got, want)
	}
}

func TestPostgresLogPath(t *testing.T) {
	t.Setenv("HOME", "/home/test")
	got := PostgresLogPath("17")
	want := "/home/test/.pv/logs/postgres-17.log"
	if got != want {
		t.Errorf("PostgresLogPath = %q, want %q", got, want)
	}
}
