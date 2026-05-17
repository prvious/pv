package scenarios

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/test/e2e/fixtures"
	"github.com/prvious/pv/test/e2e/harness"
)

func TestPvMissingInstallAndBlockedDependencyFailures(t *testing.T) {
	repoRoot := findRepoRoot(t)
	binary, err := harness.BuildActiveBinary(t.Context(), repoRoot, t.TempDir())
	if err != nil {
		t.Fatalf("build active binary: %v", err)
	}
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
	if _, err := fixture.WriteContract(
		fixtures.WithHosts("acme.test"),
		fixtures.WithServices("postgres", "mailpit", "rustfs"),
		fixtures.WithSetup("php artisan migrate"),
	); err != nil {
		t.Fatalf("write contract: %v", err)
	}

	runner := harness.CommandRunner{Executable: binary.Path, Sandbox: sandbox}
	linkResult := runner.Run(t.Context(), fixture.Root, "link")
	assertCommandFailed(t, linkResult, "pv link")
	assertEmptyStdout(t, linkResult, "pv link")
	assertContains(t, linkResult.Stderr, "PHP runtime 8.4 is not installed", "pv link")
	assertContains(t, linkResult.Stderr, "run pv php:install 8.4", "pv link")
	assertMissing(t, filepath.Join(sandbox.PVRoot, "state", "reconcile.signal"))

	resourceStatus := runner.Run(t.Context(), fixture.Root, "status", "resource")
	assertCommandSucceeded(t, resourceStatus, "pv status resource")
	assertEmptyStdout(t, resourceStatus, "pv status resource")
	for _, want := range []string{
		"resource mailpit: missing_install",
		"next action: run pv mailpit:install <version>",
		"resource postgres: missing_install",
		"next action: run pv postgres:install <version>",
		"resource rustfs: missing_install",
		"next action: run pv rustfs:install <version>",
		"AWS_SECRET_ACCESS_KEY=<redacted>",
	} {
		assertContains(t, resourceStatus.Stderr, want, "pv status resource")
	}
	assertNotContains(t, resourceStatus.Stderr, "local-rustfs-secret", "pv status resource")

	composerResult := runner.Run(t.Context(), fixture.Root, "composer:install", "2.8.0", "--php", "8.4")
	assertCommandSucceeded(t, composerResult, "pv composer:install")
	assertEmptyStdout(t, composerResult, "pv composer:install")

	runtimeStatus := runner.Run(t.Context(), fixture.Root, "status", "runtime")
	assertCommandSucceeded(t, runtimeStatus, "pv status runtime")
	assertEmptyStdout(t, runtimeStatus, "pv status runtime")
	for _, want := range []string{
		"runtime php: missing_install",
		"next action: run pv php:install 8.4",
		"runtime composer: blocked",
		"desired: composer 2.8.0 install with php 8.4",
		"observed: composer 2.8.0 blocked",
		"last error: PHP runtime 8.4 is not installed",
		"next action: run pv php:install 8.4",
	} {
		assertContains(t, runtimeStatus.Stderr, want, "pv status runtime")
	}
}

func assertCommandFailed(t *testing.T, result harness.CommandResult, command string) {
	t.Helper()

	if result.ExitCode == 0 {
		t.Fatalf("%s unexpectedly succeeded, stderr:\n%s", command, result.Stderr)
	}
	if strings.TrimSpace(result.Stderr) == "" {
		t.Fatalf("%s failed without actionable stderr", command)
	}
}
