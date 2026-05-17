package scenarios

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/test/e2e/fixtures"
	"github.com/prvious/pv/test/e2e/harness"
)

func TestPvStatusAndHelperWorkflows(t *testing.T) {
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
	if err := fixture.WriteEnv("AWS_SECRET_ACCESS_KEY=from-env-secret\nDB_PASSWORD=from-env-password\n"); err != nil {
		t.Fatalf("write existing env: %v", err)
	}
	contract, err := fixture.WriteContract(
		fixtures.WithHosts("acme.test"),
		fixtures.WithServices("postgres", "mailpit", "rustfs"),
	)
	if err != nil {
		t.Fatalf("write contract: %v", err)
	}

	runner := harness.CommandRunner{Executable: binary.Path, Sandbox: sandbox}
	linkResult := runner.Run(t.Context(), fixture.Root, "link")
	if linkResult.ExitCode != 0 {
		t.Fatalf("pv link exit = %d, stderr:\n%s", linkResult.ExitCode, linkResult.Stderr)
	}

	statusResult := runner.Run(t.Context(), fixture.Root, "status")
	assertCommandSucceeded(t, statusResult, "pv status")
	assertEmptyStdout(t, statusResult, "pv status")
	statusOutput := statusResult.Stderr
	statusProjectPath := resolvedPath(t, fixture.Root)
	for _, want := range []string{
		"project acme.test: unknown",
		"desired: path=" + statusProjectPath + " php=" + contract.PHP + " hosts=acme.test services=postgres,mailpit,rustfs",
		"observed: pending reconciliation",
		"runtime php: missing_install",
		"desired: php 8.4",
		"log: " + filepath.Join(sandbox.PVRoot, "logs", "php", "8.4.log"),
		"next action: run pv php:install 8.4",
		"resource mailpit: unknown",
		"resource postgres: unknown",
		"resource rustfs: unknown",
		"gateway acme.test: unknown",
	} {
		assertContains(t, statusOutput, want, "aggregate status")
	}
	for _, secret := range []string{"from-env-secret", "from-env-password"} {
		assertNotContains(t, statusOutput, secret, "aggregate status")
	}

	assertTargetedStatus(t, runner, fixture.Root, "project", "project acme.test:", "resource postgres:")
	assertTargetedStatus(t, runner, fixture.Root, "runtime", "runtime php:", "project acme.test:")
	assertTargetedStatus(t, runner, fixture.Root, "resource", "resource postgres:", "gateway acme.test:")
	assertTargetedStatus(t, runner, fixture.Root, "gateway", "gateway acme.test:", "runtime php:")

	nestedDir := filepath.Join(fixture.Root, "routes")
	assertHelperRoute(t, runner, nestedDir, []string{"artisan", "about", "--json"}, "run php artisan about --json")
	assertHelperRoute(t, runner, nestedDir, []string{"db", "shell"}, "run db shell")
	assertHelperRoute(t, runner, nestedDir, []string{"mail", "open"}, "run mail open")
	assertHelperRoute(t, runner, nestedDir, []string{"s3", "buckets"}, "run s3 buckets")
}

func assertTargetedStatus(t *testing.T, runner harness.CommandRunner, dir string, view string, want string, unwanted string) {
	t.Helper()

	result := runner.Run(t.Context(), dir, "status", view)
	assertCommandSucceeded(t, result, "pv status "+view)
	assertEmptyStdout(t, result, "pv status "+view)
	assertContains(t, result.Stderr, want, "pv status "+view)
	assertNotContains(t, result.Stderr, unwanted, "pv status "+view)
}

func assertHelperRoute(t *testing.T, runner harness.CommandRunner, dir string, args []string, want string) {
	t.Helper()

	result := runner.Run(t.Context(), dir, args...)
	assertCommandSucceeded(t, result, "pv "+strings.Join(args, " "))
	assertEmptyStdout(t, result, "pv "+strings.Join(args, " "))
	if got := strings.TrimSpace(result.Stderr); got != want {
		t.Fatalf("pv %s stderr = %q, want %q", strings.Join(args, " "), got, want)
	}
}

func assertCommandSucceeded(t *testing.T, result harness.CommandResult, command string) {
	t.Helper()

	if result.ExitCode != 0 {
		t.Fatalf("%s exit = %d, stderr:\n%s", command, result.ExitCode, result.Stderr)
	}
}

func assertEmptyStdout(t *testing.T, result harness.CommandResult, command string) {
	t.Helper()

	if result.Stdout != "" {
		t.Fatalf("%s stdout = %q, want empty pipeable stdout", command, result.Stdout)
	}
}

func assertContains(t *testing.T, output string, want string, label string) {
	t.Helper()

	if !strings.Contains(output, want) {
		t.Fatalf("%s missing %q:\n%s", label, want, output)
	}
}

func assertNotContains(t *testing.T, output string, unwanted string, label string) {
	t.Helper()

	if strings.Contains(output, unwanted) {
		t.Fatalf("%s included %q:\n%s", label, unwanted, output)
	}
}

func resolvedPath(t *testing.T, path string) string {
	t.Helper()

	resolved, err := filepath.EvalSymlinks(path)
	if err != nil {
		t.Fatalf("resolve %s: %v", path, err)
	}
	return resolved
}
