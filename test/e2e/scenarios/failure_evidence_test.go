package scenarios

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/prvious/pv/internal/project"
	"github.com/prvious/pv/test/e2e/fixtures"
	"github.com/prvious/pv/test/e2e/harness"
)

func TestPvSetupProcessAndGatewayFailureEvidence(t *testing.T) {
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
		fixtures.WithSetup("php artisan fixture:fail", "php artisan should:not-run"),
	); err != nil {
		t.Fatalf("write contract: %v", err)
	}

	runner := harness.CommandRunner{Executable: binary.Path, Sandbox: sandbox}
	linkResult := runner.Run(t.Context(), fixture.Root, "link")
	assertCommandFailed(t, linkResult, "pv link")
	assertEmptyStdout(t, linkResult, "pv link")
	for _, want := range []string{
		"setup command \"php artisan fixture:fail\" failed",
		"exit status 42",
		"fixture setup failed",
		"next action: fix the failing setup command and run pv link again",
	} {
		assertContains(t, linkResult.Stderr, want, "pv link")
	}
	assertMissing(t, filepath.Join(sandbox.PVRoot, "state", "reconcile.signal"))

	setupLog := readFile(t, filepath.Join(fixture.Root, "storage", "logs", "setup.log"))
	assertContains(t, setupLog, "args=artisan fixture:fail", "setup log")
	assertNotContains(t, setupLog, "should:not-run", "setup log")

	projectStatus := runner.Run(t.Context(), fixture.Root, "status", "project")
	assertCommandSucceeded(t, projectStatus, "pv status project")
	assertFailureEvidence(t, projectStatus.Stderr, failureExpectation{
		Header:     "project acme.test: failed",
		Scenario:   "setup",
		Command:    "php artisan fixture:fail",
		Expected:   "setup commands complete",
		Actual:     "exit status 42: fixture setup failed",
		LogPath:    filepath.Join(sandbox.PVRoot, "logs", "setup", "acme.test.log"),
		NextAction: "fix the failing setup command and run pv link again",
	})

	registry := project.Registry{Path: filepath.Join(sandbox.PVRoot, "state", "project.json")}
	recordFakeFailure(t, registry, project.Failure{
		View:       "resource",
		Name:       "mailpit",
		Scenario:   "process",
		Command:    "mailpit --smtp 127.0.0.1:1025 --listen 127.0.0.1:8025",
		Expected:   "mailpit process remains running",
		Actual:     "process exited with code 42",
		LogPath:    filepath.Join(sandbox.PVRoot, "logs", "mailpit", "crash.log"),
		NextAction: "inspect the Mailpit log and run reconciliation again",
	})
	resourceStatus := runner.Run(t.Context(), fixture.Root, "status", "resource")
	assertCommandSucceeded(t, resourceStatus, "pv status resource")
	assertFailureEvidence(t, resourceStatus.Stderr, failureExpectation{
		Header:     "resource mailpit: failed",
		Scenario:   "process",
		Command:    "mailpit --smtp 127.0.0.1:1025 --listen 127.0.0.1:8025",
		Expected:   "mailpit process remains running",
		Actual:     "process exited with code 42",
		LogPath:    filepath.Join(sandbox.PVRoot, "logs", "mailpit", "crash.log"),
		NextAction: "inspect the Mailpit log and run reconciliation again",
	})

	recordFakeFailure(t, registry, project.Failure{
		View:       "gateway",
		Name:       "acme.test",
		Scenario:   "gateway",
		Command:    "apply route acme.test",
		Expected:   "gateway route applied without host mutation",
		Actual:     "fake TLS adapter refused certificate",
		LogPath:    filepath.Join(sandbox.PVRoot, "logs", "gateway", "acme.test.failure.log"),
		NextAction: "fix the gateway route failure and run reconciliation again",
	})
	gatewayStatus := runner.Run(t.Context(), fixture.Root, "status", "gateway")
	assertCommandSucceeded(t, gatewayStatus, "pv status gateway")
	assertFailureEvidence(t, gatewayStatus.Stderr, failureExpectation{
		Header:     "gateway acme.test: failed",
		Scenario:   "gateway",
		Command:    "apply route acme.test",
		Expected:   "gateway route applied without host mutation",
		Actual:     "fake TLS adapter refused certificate",
		LogPath:    filepath.Join(sandbox.PVRoot, "logs", "gateway", "acme.test.failure.log"),
		NextAction: "fix the gateway route failure and run reconciliation again",
	})

	state, ok, err := registry.Current(t.Context())
	if err != nil {
		t.Fatalf("read project state: %v", err)
	}
	if !ok || len(state.Hosts) != 1 || state.Hosts[0] != "acme.test" {
		t.Fatalf("gateway failure mutated hosts: %#v", state.Hosts)
	}
}

type failureExpectation struct {
	Header     string
	Scenario   string
	Command    string
	Expected   string
	Actual     string
	LogPath    string
	NextAction string
}

func assertFailureEvidence(t *testing.T, output string, expected failureExpectation) {
	t.Helper()

	for _, want := range []string{
		expected.Header,
		"scenario=" + expected.Scenario,
		"command=" + expected.Command,
		"expected=" + expected.Expected,
		"actual=" + expected.Actual,
		"log: " + expected.LogPath,
		"last error: " + expected.Actual,
		"next action: " + expected.NextAction,
	} {
		assertContains(t, output, want, "failure evidence")
	}
}

func recordFakeFailure(t *testing.T, registry project.Registry, failure project.Failure) {
	t.Helper()

	if err := os.MkdirAll(filepath.Dir(failure.LogPath), 0o755); err != nil {
		t.Fatalf("create fake failure log dir: %v", err)
	}
	if err := os.WriteFile(failure.LogPath, []byte(failure.Actual+"\n"), 0o600); err != nil {
		t.Fatalf("write fake failure log: %v", err)
	}
	if err := registry.RecordFailure(t.Context(), failure); err != nil {
		t.Fatalf("record fake failure: %v", err)
	}
}

func installFailingPHP(sandbox *harness.Sandbox) error {
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
  printf 'args=%s\n' "$*"
} >> storage/logs/setup.log
if [ "$*" = "artisan fixture:fail" ]; then
  printf 'fixture setup failed\n' >&2
  exit 42
fi
exit 0
`
	return os.WriteFile(filepath.Join(binDir, "php"), []byte(script), 0o755)
}
