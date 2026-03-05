package phpenv

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// fakePHP is a bash script that acts as a stand-in for the real PHP binary.
// When invoked, it writes the environment and arguments to a capture file
// so tests can inspect exactly what the composer shim passed through.
const fakePHP = `#!/bin/bash
CAPTURE_FILE="${FAKE_PHP_CAPTURE:-/dev/null}"
{
  echo "COMPOSER_HOME=$COMPOSER_HOME"
  echo "COMPOSER_CACHE_DIR=$COMPOSER_CACHE_DIR"
  echo "ARGS=$*"
  echo "PHP_VERSION=8.4"
} > "$CAPTURE_FILE"
`

// fakeComposerPhar is a minimal PHP script that prints Composer-like output.
// When the fake PHP receives it as the first argument, the test can verify
// the phar path was passed correctly.
const fakeComposerPhar = `<?php echo "composer-phar-placeholder";`

// setupE2E creates a full ~/.pv directory structure in a temp dir with:
//   - settings.json with global PHP 8.4
//   - a fake php binary at ~/.pv/php/8.4/php
//   - a placeholder composer.phar at ~/.pv/data/composer.phar
//   - the real shims written by WriteShims()
//
// Returns the temp home dir and a capture file path for reading fake PHP output.
func setupE2E(t *testing.T) (home string, captureFile string) {
	t.Helper()
	home = t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Write settings with global PHP version.
	settings := &config.Settings{TLD: "test", GlobalPHP: "8.4"}
	if err := settings.Save(); err != nil {
		t.Fatal(err)
	}

	// Create fake PHP binary.
	phpDir := filepath.Join(config.PhpDir(), "8.4")
	os.MkdirAll(phpDir, 0755)
	if err := os.WriteFile(filepath.Join(phpDir, "php"), []byte(fakePHP), 0755); err != nil {
		t.Fatal(err)
	}

	// Create placeholder composer.phar.
	if err := os.WriteFile(config.ComposerPharPath(), []byte(fakeComposerPhar), 0644); err != nil {
		t.Fatal(err)
	}

	// Write the real shims.
	if err := WriteShims(); err != nil {
		t.Fatal(err)
	}

	captureFile = filepath.Join(home, "capture.txt")
	return home, captureFile
}

// runComposerShim executes the composer shim with the given args.
// It sets FAKE_PHP_CAPTURE so the fake PHP binary writes its env/args to captureFile.
// If dir is non-empty, the shim runs in that directory (for version resolution tests).
func runComposerShim(t *testing.T, captureFile string, dir string, args ...string) (stdout string, stderr string) {
	t.Helper()
	composerShim := filepath.Join(config.BinDir(), "composer")
	cmd := exec.Command(composerShim, args...)
	cmd.Env = append(os.Environ(),
		"HOME="+os.Getenv("HOME"),
		"FAKE_PHP_CAPTURE="+captureFile,
	)
	if dir != "" {
		cmd.Dir = dir
	}
	var outBuf, errBuf strings.Builder
	cmd.Stdout = &outBuf
	cmd.Stderr = &errBuf
	if err := cmd.Run(); err != nil {
		// If fake PHP was reached, Run() succeeds. If shim itself errors, we may
		// want to inspect stderr. Return both.
		return outBuf.String(), errBuf.String()
	}
	return outBuf.String(), errBuf.String()
}

// readCapture reads the capture file written by the fake PHP binary and returns
// a map of KEY=VALUE pairs.
func readCapture(t *testing.T, captureFile string) map[string]string {
	t.Helper()
	data, err := os.ReadFile(captureFile)
	if err != nil {
		t.Fatalf("cannot read capture file: %v", err)
	}
	result := make(map[string]string)
	for _, line := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			result[parts[0]] = parts[1]
		}
	}
	return result
}

func TestE2E_ComposerShim_SetsComposerHome(t *testing.T) {
	_, captureFile := setupE2E(t)

	runComposerShim(t, captureFile, "", "install")
	env := readCapture(t, captureFile)

	want := config.ComposerDir()
	if env["COMPOSER_HOME"] != want {
		t.Errorf("COMPOSER_HOME = %q, want %q", env["COMPOSER_HOME"], want)
	}
}

func TestE2E_ComposerShim_SetsComposerCacheDir(t *testing.T) {
	_, captureFile := setupE2E(t)

	runComposerShim(t, captureFile, "", "install")
	env := readCapture(t, captureFile)

	want := config.ComposerCacheDir()
	if env["COMPOSER_CACHE_DIR"] != want {
		t.Errorf("COMPOSER_CACHE_DIR = %q, want %q", env["COMPOSER_CACHE_DIR"], want)
	}
}

func TestE2E_ComposerShim_PassesPharAsFirstArg(t *testing.T) {
	_, captureFile := setupE2E(t)

	runComposerShim(t, captureFile, "", "global", "require", "laravel/installer")
	env := readCapture(t, captureFile)

	args := env["ARGS"]
	pharPath := config.ComposerPharPath()
	if !strings.HasPrefix(args, pharPath) {
		t.Errorf("ARGS = %q, want to start with phar path %q", args, pharPath)
	}
}

func TestE2E_ComposerShim_ForwardsAllArgs(t *testing.T) {
	_, captureFile := setupE2E(t)

	runComposerShim(t, captureFile, "", "global", "require", "laravel/installer", "--dev")
	env := readCapture(t, captureFile)

	args := env["ARGS"]
	for _, want := range []string{"global", "require", "laravel/installer", "--dev"} {
		if !strings.Contains(args, want) {
			t.Errorf("ARGS = %q, missing %q", args, want)
		}
	}
}

func TestE2E_ComposerShim_ResolvesGlobalPHP(t *testing.T) {
	_, captureFile := setupE2E(t)

	runComposerShim(t, captureFile, "", "about")
	env := readCapture(t, captureFile)

	if env["PHP_VERSION"] != "8.4" {
		t.Errorf("PHP_VERSION = %q, want 8.4 (global default)", env["PHP_VERSION"])
	}
}

func TestE2E_ComposerShim_ResolvesPvPhpFile(t *testing.T) {
	home, captureFile := setupE2E(t)

	// Create a second PHP version.
	php83Dir := filepath.Join(config.PhpDir(), "8.3")
	os.MkdirAll(php83Dir, 0755)
	fakePHP83 := strings.Replace(fakePHP, "PHP_VERSION=8.4", "PHP_VERSION=8.3", 1)
	os.WriteFile(filepath.Join(php83Dir, "php"), []byte(fakePHP83), 0755)

	// Create a project dir with .pv-php pointing to 8.3.
	projectDir := filepath.Join(home, "my-project")
	os.MkdirAll(projectDir, 0755)
	os.WriteFile(filepath.Join(projectDir, ".pv-php"), []byte("8.3"), 0644)

	runComposerShim(t, captureFile, projectDir, "install")
	env := readCapture(t, captureFile)

	if env["PHP_VERSION"] != "8.3" {
		t.Errorf("PHP_VERSION = %q, want 8.3 (from .pv-php)", env["PHP_VERSION"])
	}
}

func TestE2E_ComposerShim_ResolvesComposerJsonConstraint(t *testing.T) {
	home, captureFile := setupE2E(t)

	// Create a second PHP version.
	php83Dir := filepath.Join(config.PhpDir(), "8.3")
	os.MkdirAll(php83Dir, 0755)
	fakePHP83 := strings.Replace(fakePHP, "PHP_VERSION=8.4", "PHP_VERSION=8.3", 1)
	os.WriteFile(filepath.Join(php83Dir, "php"), []byte(fakePHP83), 0755)

	// Create a project with composer.json requiring PHP 8.3.
	projectDir := filepath.Join(home, "laravel-app")
	os.MkdirAll(projectDir, 0755)
	composerJSON := map[string]interface{}{
		"require": map[string]string{
			"php": "^8.3",
		},
	}
	data, _ := json.Marshal(composerJSON)
	os.WriteFile(filepath.Join(projectDir, "composer.json"), data, 0644)

	runComposerShim(t, captureFile, projectDir, "install")
	env := readCapture(t, captureFile)

	if env["PHP_VERSION"] != "8.3" {
		t.Errorf("PHP_VERSION = %q, want 8.3 (from composer.json ^8.3)", env["PHP_VERSION"])
	}
}

func TestE2E_ComposerShim_NeverTouchesSystemComposer(t *testing.T) {
	home, captureFile := setupE2E(t)

	// Create a ~/.composer dir (the system one) and put a sentinel file in it.
	systemComposerDir := filepath.Join(home, ".composer")
	os.MkdirAll(systemComposerDir, 0755)
	sentinel := filepath.Join(systemComposerDir, "sentinel.txt")
	os.WriteFile(sentinel, []byte("before"), 0644)

	runComposerShim(t, captureFile, "", "install")
	env := readCapture(t, captureFile)

	// COMPOSER_HOME must NOT be ~/.composer.
	if env["COMPOSER_HOME"] == systemComposerDir {
		t.Error("COMPOSER_HOME points to ~/.composer — isolation is broken")
	}
	if !strings.Contains(env["COMPOSER_HOME"], filepath.Join(".pv", "composer")) {
		t.Errorf("COMPOSER_HOME = %q, want to contain .pv/composer", env["COMPOSER_HOME"])
	}

	// Sentinel should be untouched.
	data, err := os.ReadFile(sentinel)
	if err != nil || string(data) != "before" {
		t.Error("~/.composer was modified during shim execution")
	}
}

func TestE2E_ComposerShim_GlobalInstallWritesToPvComposer(t *testing.T) {
	_, captureFile := setupE2E(t)

	runComposerShim(t, captureFile, "", "global", "require", "laravel/installer")
	env := readCapture(t, captureFile)

	// The COMPOSER_HOME should be ~/.pv/composer so that `composer global require`
	// writes packages into ~/.pv/composer/vendor/bin.
	if env["COMPOSER_HOME"] != config.ComposerDir() {
		t.Errorf("COMPOSER_HOME = %q, want %q — global installs would go to wrong dir",
			env["COMPOSER_HOME"], config.ComposerDir())
	}
}

func TestE2E_ComposerShim_CacheDirIsolated(t *testing.T) {
	_, captureFile := setupE2E(t)

	runComposerShim(t, captureFile, "", "install")
	env := readCapture(t, captureFile)

	want := config.ComposerCacheDir()
	if env["COMPOSER_CACHE_DIR"] != want {
		t.Errorf("COMPOSER_CACHE_DIR = %q, want %q", env["COMPOSER_CACHE_DIR"], want)
	}
	// Verify the cache dir actually exists on disk.
	if info, err := os.Stat(want); err != nil || !info.IsDir() {
		t.Errorf("COMPOSER_CACHE_DIR %q does not exist as a directory", want)
	}
}

func TestE2E_ComposerShim_FailsWithoutPHPVersion(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Write settings with NO global PHP.
	settings := &config.Settings{TLD: "test"}
	settings.Save()

	if err := WriteShims(); err != nil {
		t.Fatal(err)
	}

	composerShim := filepath.Join(config.BinDir(), "composer")
	cmd := exec.Command(composerShim, "install")
	cmd.Env = append(os.Environ(), "HOME="+home)
	// Run in a dir with no .pv-php or composer.json.
	cmd.Dir = home
	output, err := cmd.CombinedOutput()

	if err == nil {
		t.Fatal("expected error when no PHP version is configured, but shim succeeded")
	}
	if !strings.Contains(string(output), "no PHP version configured") {
		t.Errorf("stderr = %q, want 'no PHP version configured'", string(output))
	}
}

func TestE2E_ComposerShim_FailsWithMissingPHPBinary(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	// Write settings pointing to a version that doesn't have a binary.
	settings := &config.Settings{TLD: "test", GlobalPHP: "8.4"}
	settings.Save()

	// Create the version dir but don't put a php binary in it.
	os.MkdirAll(filepath.Join(config.PhpDir(), "8.4"), 0755)

	if err := WriteShims(); err != nil {
		t.Fatal(err)
	}

	composerShim := filepath.Join(config.BinDir(), "composer")
	cmd := exec.Command(composerShim, "install")
	cmd.Env = append(os.Environ(), "HOME="+home)
	cmd.Dir = home
	output, err := cmd.CombinedOutput()

	if err == nil {
		t.Fatal("expected error when PHP binary is missing, but shim succeeded")
	}
	if !strings.Contains(string(output), "not installed") {
		t.Errorf("stderr = %q, want 'not installed'", string(output))
	}
}

func TestE2E_ComposerShim_OverridesExternalComposerEnv(t *testing.T) {
	_, captureFile := setupE2E(t)

	// Set external COMPOSER_HOME to something else — the shim should override it.
	composerShim := filepath.Join(config.BinDir(), "composer")
	cmd := exec.Command(composerShim, "install")
	cmd.Env = append(os.Environ(),
		"HOME="+os.Getenv("HOME"),
		"FAKE_PHP_CAPTURE="+captureFile,
		"COMPOSER_HOME=/tmp/evil-composer-home",
		"COMPOSER_CACHE_DIR=/tmp/evil-cache",
	)
	cmd.Run()
	env := readCapture(t, captureFile)

	if env["COMPOSER_HOME"] == "/tmp/evil-composer-home" {
		t.Error("shim did not override external COMPOSER_HOME — isolation is broken")
	}
	if env["COMPOSER_HOME"] != config.ComposerDir() {
		t.Errorf("COMPOSER_HOME = %q, want %q", env["COMPOSER_HOME"], config.ComposerDir())
	}

	if env["COMPOSER_CACHE_DIR"] == "/tmp/evil-cache" {
		t.Error("shim did not override external COMPOSER_CACHE_DIR — isolation is broken")
	}
	if env["COMPOSER_CACHE_DIR"] != config.ComposerCacheDir() {
		t.Errorf("COMPOSER_CACHE_DIR = %q, want %q", env["COMPOSER_CACHE_DIR"], config.ComposerCacheDir())
	}
}
