package scenarios

import (
	"testing"

	"github.com/prvious/pv/test/e2e/fixtures"
	"github.com/prvious/pv/test/e2e/harness"
)

func TestPvRecoveryAfterCorrectiveAction(t *testing.T) {
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
	if err := installFailingPHP(sandbox); err != nil {
		t.Fatalf("install failing php: %v", err)
	}
	if _, err := fixture.WriteContract(
		fixtures.WithHosts("acme.test"),
		fixtures.WithServices("mailpit"),
		fixtures.WithSetup("php artisan fixture:fail"),
	); err != nil {
		t.Fatalf("write failing contract: %v", err)
	}

	runner := harness.CommandRunner{Executable: binary.Path, Sandbox: sandbox}
	failedLink := runner.Run(t.Context(), fixture.Root, "link")
	assertCommandFailed(t, failedLink, "pv link")
	beforeProject := runner.Run(t.Context(), fixture.Root, "status", "project")
	assertCommandSucceeded(t, beforeProject, "pv status project before recovery")
	assertContains(t, beforeProject.Stderr, "project acme.test: failed", "project status before recovery")
	assertContains(t, beforeProject.Stderr, "actual=exit status 42: fixture setup failed", "project status before recovery")

	if _, err := fixture.WriteContract(
		fixtures.WithHosts("acme.test"),
		fixtures.WithServices("mailpit"),
		fixtures.WithSetup("php artisan fixture:ok"),
	); err != nil {
		t.Fatalf("write recovered contract: %v", err)
	}
	recoveredLink := runner.Run(t.Context(), fixture.Root, "link")
	assertCommandSucceeded(t, recoveredLink, "pv link after setup recovery")
	afterProject := runner.Run(t.Context(), fixture.Root, "status", "project")
	assertCommandSucceeded(t, afterProject, "pv status project after recovery")
	assertContains(t, afterProject.Stderr, "project acme.test: unknown", "project status after recovery")
	assertContains(t, afterProject.Stderr, "observed: pending reconciliation", "project status after recovery")
	for _, stale := range []string{"fixture setup failed", "scenario=setup", "project acme.test: failed"} {
		assertNotContains(t, afterProject.Stderr, stale, "project status after recovery")
	}

	composerResult := runner.Run(t.Context(), fixture.Root, "composer:install", "2.8.0", "--php", "8.4")
	assertCommandSucceeded(t, composerResult, "pv composer:install")
	beforeRuntime := runner.Run(t.Context(), fixture.Root, "status", "runtime")
	assertCommandSucceeded(t, beforeRuntime, "pv status runtime before recovery")
	assertContains(t, beforeRuntime.Stderr, "runtime composer: blocked", "runtime status before recovery")
	assertContains(t, beforeRuntime.Stderr, "last error: PHP runtime 8.4 is not installed", "runtime status before recovery")

	phpResult := runner.Run(t.Context(), fixture.Root, "php:install", "8.4")
	assertCommandSucceeded(t, phpResult, "pv php:install")
	afterRuntime := runner.Run(t.Context(), fixture.Root, "status", "runtime")
	assertCommandSucceeded(t, afterRuntime, "pv status runtime after recovery")
	for _, want := range []string{
		"runtime composer: unknown",
		"observed: composer pending",
		"desired: php 8.4 install",
	} {
		assertContains(t, afterRuntime.Stderr, want, "runtime status after recovery")
	}
	for _, stale := range []string{"runtime composer: blocked", "PHP runtime 8.4 is not installed"} {
		assertNotContains(t, afterRuntime.Stderr, stale, "runtime status after recovery")
	}
}
