package phpenv

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// seedIniDevelopment drops the testdata fixture into etc/php.ini-development
// for the given version, mirroring what the install code does.
func seedIniDevelopment(t *testing.T, version string) {
	t.Helper()
	src, err := os.ReadFile("testdata/php.ini-development")
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	dst := filepath.Join(config.PhpEtcDir(version), "php.ini-development")
	if err := os.MkdirAll(filepath.Dir(dst), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(dst, src, 0644); err != nil {
		t.Fatal(err)
	}
}

func TestEnsureIniLayout_CreatesAllDirsAndFiles(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	// Dirs.
	for _, dir := range []string{
		config.PhpEtcDir("8.4"),
		config.PhpConfDDir("8.4"),
		config.PhpSessionDir("8.4"),
		config.PhpTmpDir("8.4"),
	} {
		info, err := os.Stat(dir)
		if err != nil {
			t.Errorf("dir %s missing: %v", dir, err)
			continue
		}
		if !info.IsDir() {
			t.Errorf("%s exists but is not a dir", dir)
		}
	}

	// php.ini was copied from php.ini-development.
	iniPath := filepath.Join(config.PhpEtcDir("8.4"), "php.ini")
	got, err := os.ReadFile(iniPath)
	if err != nil {
		t.Fatalf("read php.ini: %v", err)
	}
	if !strings.Contains(string(got), "memory_limit = 128M") {
		t.Errorf("php.ini does not contain fixture content; got: %q", string(got))
	}

	// 00-pv.ini was written and contains the expected directives.
	pvIniPath := filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini")
	pvIni, err := os.ReadFile(pvIniPath)
	if err != nil {
		t.Fatalf("read 00-pv.ini: %v", err)
	}
	wantSession := "session.save_path = \"" + config.PhpSessionDir("8.4") + "\""
	if !strings.Contains(string(pvIni), wantSession) {
		t.Errorf("00-pv.ini missing %q; got:\n%s", wantSession, string(pvIni))
	}
	if !strings.Contains(string(pvIni), "date.timezone = UTC") {
		t.Error("00-pv.ini missing date.timezone")
	}
}

func TestEnsureIniLayout_PreservesExistingPhpIni(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	// Pre-create php.ini with user content; EnsureIniLayout must not overwrite.
	iniPath := filepath.Join(config.PhpEtcDir("8.4"), "php.ini")
	if err := os.MkdirAll(filepath.Dir(iniPath), 0755); err != nil {
		t.Fatal(err)
	}
	userContent := "; user-edited php.ini\nmemory_limit = 1G\n"
	if err := os.WriteFile(iniPath, []byte(userContent), 0644); err != nil {
		t.Fatal(err)
	}

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	got, err := os.ReadFile(iniPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != userContent {
		t.Errorf("php.ini was clobbered; got:\n%s\nwant:\n%s", string(got), userContent)
	}
}

func TestEnsureIniLayout_RegeneratesPvIni(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	// Pre-create 00-pv.ini with stale content; EnsureIniLayout must overwrite.
	pvIniPath := filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini")
	if err := os.MkdirAll(filepath.Dir(pvIniPath), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(pvIniPath, []byte("; stale junk\n"), 0644); err != nil {
		t.Fatal(err)
	}

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	got, err := os.ReadFile(pvIniPath)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(got), "stale junk") {
		t.Errorf("00-pv.ini was not regenerated; got:\n%s", string(got))
	}
	if !strings.Contains(string(got), "date.timezone = UTC") {
		t.Errorf("regenerated 00-pv.ini missing canonical content; got:\n%s", string(got))
	}
}

func TestEnsureIniLayout_Idempotent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("first EnsureIniLayout error = %v", err)
	}
	iniPath := filepath.Join(config.PhpEtcDir("8.4"), "php.ini")
	pvIniPath := filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini")
	iniAfter1, err := os.ReadFile(iniPath)
	if err != nil {
		t.Fatal(err)
	}
	pvIniAfter1, err := os.ReadFile(pvIniPath)
	if err != nil {
		t.Fatal(err)
	}

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("second EnsureIniLayout error = %v", err)
	}
	iniAfter2, err := os.ReadFile(iniPath)
	if err != nil {
		t.Fatal(err)
	}
	pvIniAfter2, err := os.ReadFile(pvIniPath)
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Equal(iniAfter1, iniAfter2) {
		t.Errorf("php.ini content changed across calls; first:\n%s\nsecond:\n%s", iniAfter1, iniAfter2)
	}
	if !bytes.Equal(pvIniAfter1, pvIniAfter2) {
		t.Errorf("00-pv.ini content changed across calls; first:\n%s\nsecond:\n%s", pvIniAfter1, pvIniAfter2)
	}
}

func TestEnsureIniLayout_VersionThreaded(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.3")
	seedIniDevelopment(t, "8.4")

	if err := EnsureIniLayout("8.3"); err != nil {
		t.Fatalf("EnsureIniLayout(8.3) error = %v", err)
	}
	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout(8.4) error = %v", err)
	}

	pv83, err := os.ReadFile(filepath.Join(config.PhpConfDDir("8.3"), "00-pv.ini"))
	if err != nil {
		t.Fatal(err)
	}
	pv84, err := os.ReadFile(filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini"))
	if err != nil {
		t.Fatal(err)
	}

	if !strings.Contains(string(pv83), config.PhpSessionDir("8.3")) {
		t.Errorf("8.3's 00-pv.ini missing its own session dir; got:\n%s", pv83)
	}
	if strings.Contains(string(pv83), config.PhpSessionDir("8.4")) {
		t.Errorf("8.3's 00-pv.ini contains 8.4's session dir; got:\n%s", pv83)
	}
	if !strings.Contains(string(pv84), config.PhpSessionDir("8.4")) {
		t.Errorf("8.4's 00-pv.ini missing its own session dir; got:\n%s", pv84)
	}
}

func TestEnsureIniLayout_NoIniDevelopmentSource(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	// Deliberately do NOT seed php.ini-development — simulating an old install.

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	// Dirs and 00-pv.ini still created.
	if _, err := os.Stat(config.PhpConfDDir("8.4")); err != nil {
		t.Errorf("conf.d not created: %v", err)
	}
	if _, err := os.Stat(filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini")); err != nil {
		t.Errorf("00-pv.ini not written: %v", err)
	}
	// But php.ini is NOT created (no source to copy from).
	iniPath := filepath.Join(config.PhpEtcDir("8.4"), "php.ini")
	if _, err := os.Stat(iniPath); !os.IsNotExist(err) {
		t.Errorf("php.ini should not exist when source is missing; got err=%v", err)
	}
}
