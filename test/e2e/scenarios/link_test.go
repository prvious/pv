package scenarios

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/prvious/pv/test/e2e/fixtures"
	"github.com/prvious/pv/test/e2e/harness"
)

func TestPvLinkEnvSetupLifecycle(t *testing.T) {
	repoRoot := findRepoRoot(t)
	binary, err := harness.BuildActiveBinary(t.Context(), repoRoot, t.TempDir())
	if err != nil {
		t.Fatalf("build active binary: %v", err)
	}

	invalidSandbox, err := harness.NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new invalid sandbox: %v", err)
	}
	defer func() {
		if err := invalidSandbox.Cleanup(); err != nil {
			t.Fatalf("cleanup invalid sandbox: %v", err)
		}
	}()
	invalidFixture, err := fixtures.NewLaravel(invalidSandbox)
	if err != nil {
		t.Fatalf("new invalid fixture: %v", err)
	}
	if err := os.WriteFile(filepath.Join(invalidFixture.Root, "pv.yml"), []byte("version: 1\nhosts:\n  - broken.test\n"), 0o644); err != nil {
		t.Fatalf("write invalid pv.yml: %v", err)
	}
	invalidRunner := harness.CommandRunner{Executable: binary.Path, Sandbox: invalidSandbox}
	invalidResult := invalidRunner.Run(t.Context(), invalidFixture.Root, "link")
	if invalidResult.ExitCode == 0 {
		t.Fatal("pv link accepted invalid pv.yml")
	}
	assertMissing(t, filepath.Join(invalidSandbox.PVRoot, "state", "project.json"))
	assertMissing(t, filepath.Join(invalidFixture.Root, ".env"))
	assertMissing(t, filepath.Join(invalidSandbox.PVRoot, "state", "reconcile.signal"))

	sandbox, err := harness.NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}
	defer func() {
		if err := sandbox.Cleanup(); err != nil {
			t.Fatalf("cleanup sandbox: %v", err)
		}
	}()
	fixture, err := fixtures.NewLaravel(sandbox, fixtures.WithName("Acme"))
	if err != nil {
		t.Fatalf("new laravel fixture: %v", err)
	}
	if err := installFakePHP(sandbox); err != nil {
		t.Fatalf("install fake php: %v", err)
	}
	if err := fixture.WriteEnv("APP_NAME=Existing\nDB_PASSWORD=user-owned-secret\n"); err != nil {
		t.Fatalf("write existing env: %v", err)
	}
	contract, err := fixture.WriteContract(
		fixtures.WithHosts("acme.test"),
		fixtures.WithServices("postgres", "mailpit"),
		fixtures.WithSetup("php artisan fixture:setup"),
	)
	if err != nil {
		t.Fatalf("write contract: %v", err)
	}

	runner := harness.CommandRunner{Executable: binary.Path, Sandbox: sandbox}
	linkResult := runner.Run(t.Context(), fixture.Root, "link")
	if linkResult.ExitCode != 0 {
		t.Fatalf("pv link exit = %d, stderr:\n%s", linkResult.ExitCode, linkResult.Stderr)
	}
	if linkResult.Stdout != "" {
		t.Fatalf("pv link stdout = %q, want empty pipeable stdout", linkResult.Stdout)
	}
	if !strings.Contains(linkResult.Stderr, "linked project") {
		t.Fatalf("pv link stderr missing status, got %q", linkResult.Stderr)
	}

	statePath := filepath.Join(sandbox.PVRoot, "state", "project.json")
	state := readProjectState(t, statePath)
	if !samePath(t, state.Path, fixture.Root) || state.PHP != contract.PHP {
		t.Fatalf("project state = %#v, want path %s php %s", state, fixture.Root, contract.PHP)
	}
	if !slices.Equal(state.Hosts, contract.Hosts) || !slices.Equal(state.Services, contract.Services) {
		t.Fatalf("project state = %#v, want hosts %#v services %#v", state, contract.Hosts, contract.Services)
	}

	env := readFile(t, filepath.Join(fixture.Root, ".env"))
	for _, want := range []string{
		"APP_NAME=Existing",
		"DB_PASSWORD=user-owned-secret",
		"# pv managed begin",
		"PV_PHP=8.4",
		"DB_CONNECTION=pgsql",
		"DB_HOST=127.0.0.1",
		"MAIL_MAILER=smtp",
		"# pv managed end",
	} {
		if !strings.Contains(env, want) {
			t.Fatalf(".env missing %q:\n%s", want, env)
		}
	}
	managed := managedEnvBlock(t, env)
	for _, unwanted := range []string{"REDIS_HOST", "AWS_ENDPOINT_URL", "DB_PASSWORD"} {
		if strings.Contains(managed, unwanted) {
			t.Fatalf("managed env block contains undeclared/user-owned key %q:\n%s", unwanted, managed)
		}
	}

	setupLog := readFile(t, filepath.Join(fixture.Root, "storage", "logs", "setup.log"))
	if !samePath(t, setupLogValue(setupLog, "cwd"), fixture.Root) {
		t.Fatalf("setup did not run from project root:\n%s", setupLog)
	}
	if !strings.Contains(setupLog, "args=artisan fixture:setup") {
		t.Fatalf("setup did not invoke artisan args:\n%s", setupLog)
	}
	if got, want := firstPathEntry(setupLog), filepath.Join(sandbox.PVRoot, "bin"); got != want {
		t.Fatalf("setup PATH first entry = %s, want %s\nlog:\n%s", got, want, setupLog)
	}

	signalPath := filepath.Join(sandbox.PVRoot, "state", "reconcile.signal")
	signal := readFile(t, signalPath)
	if !strings.Contains(signal, statePath) {
		t.Fatalf("signal did not include durable state path %s:\n%s", statePath, signal)
	}

	_, err = fixture.WriteContract(
		fixtures.WithHosts("acme.test"),
		fixtures.WithServices("mailpit"),
		fixtures.WithSetup("php artisan fixture:setup"),
	)
	if err != nil {
		t.Fatalf("write contract without postgres: %v", err)
	}
	relink := runner.Run(t.Context(), fixture.Root, "link")
	if relink.ExitCode != 0 {
		t.Fatalf("pv relink exit = %d, stderr:\n%s", relink.ExitCode, relink.Stderr)
	}
	env = readFile(t, filepath.Join(fixture.Root, ".env"))
	if !strings.Contains(env, "DB_PASSWORD=user-owned-secret") {
		t.Fatalf("relink mutated user-owned env key:\n%s", env)
	}
	if strings.Contains(managedEnvBlock(t, env), "DB_CONNECTION=pgsql") {
		t.Fatalf("relink kept removed postgres declaration in managed block:\n%s", env)
	}
}

type projectState struct {
	Path     string   `json:"path"`
	PHP      string   `json:"php"`
	Hosts    []string `json:"hosts"`
	Services []string `json:"services"`
}

func readProjectState(t *testing.T, path string) projectState {
	t.Helper()

	var state projectState
	if err := json.Unmarshal([]byte(readFile(t, path)), &state); err != nil {
		t.Fatalf("decode project state %s: %v", path, err)
	}
	return state
}

func installFakePHP(sandbox *harness.Sandbox) error {
	binDir := filepath.Join(sandbox.PVRoot, "bin")
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		return err
	}
	runtimeDir := filepath.Join(sandbox.PVRoot, "runtimes", "php", "8.4")
	if err := os.MkdirAll(runtimeDir, 0o755); err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(runtimeDir, "installed"), []byte("php 8.4\n"), 0o644); err != nil {
		return err
	}
	script := `#!/bin/sh
# php 8.4
{
  printf 'cwd=%s\n' "$(pwd)"
  printf 'path=%s\n' "$PATH"
  printf 'args=%s\n' "$*"
} > storage/logs/setup.log
`
	return os.WriteFile(filepath.Join(binDir, "php"), []byte(script), 0o755)
}

func managedEnvBlock(t *testing.T, env string) string {
	t.Helper()

	start := strings.Index(env, "# pv managed begin")
	end := strings.Index(env, "# pv managed end")
	if start == -1 || end == -1 || end < start {
		t.Fatalf("missing managed env block:\n%s", env)
	}
	return env[start:end]
}

func firstPathEntry(setupLog string) string {
	entry, _, _ := strings.Cut(setupLogValue(setupLog, "path"), string(os.PathListSeparator))
	return entry
}

func setupLogValue(setupLog string, key string) string {
	for _, line := range strings.Split(setupLog, "\n") {
		if value, ok := strings.CutPrefix(line, key+"="); ok {
			return value
		}
	}
	return ""
}

func samePath(t *testing.T, left string, right string) bool {
	t.Helper()

	leftReal, err := filepath.EvalSymlinks(left)
	if err != nil {
		t.Fatalf("eval symlinks %s: %v", left, err)
	}
	rightReal, err := filepath.EvalSymlinks(right)
	if err != nil {
		t.Fatalf("eval symlinks %s: %v", right, err)
	}
	return leftReal == rightReal
}
