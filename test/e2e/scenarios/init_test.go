package scenarios

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/project"
	"github.com/prvious/pv/test/e2e/fixtures"
	"github.com/prvious/pv/test/e2e/harness"
)

func TestPvInitLifecycle(t *testing.T) {
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
	runner := harness.CommandRunner{Executable: binary.Path, Sandbox: sandbox}

	initResult := runner.Run(t.Context(), fixture.Root, "init")
	if initResult.ExitCode != 0 {
		t.Fatalf("pv init exit = %d, stderr:\n%s", initResult.ExitCode, initResult.Stderr)
	}
	if initResult.Stdout != "" {
		t.Fatalf("pv init stdout = %q, want empty pipeable stdout", initResult.Stdout)
	}
	if !strings.Contains(initResult.Stderr, "created pv.yml") {
		t.Fatalf("pv init stderr missing status, got %q", initResult.Stderr)
	}

	pvYMLPath := filepath.Join(fixture.Root, "pv.yml")
	generated := readFile(t, pvYMLPath)
	expected := project.DefaultLaravelContract(filepath.Base(fixture.Root)).String()
	if generated != expected {
		t.Fatalf("generated pv.yml =\n%s\nwant:\n%s", generated, expected)
	}
	if !strings.Contains(generated, "version: 1\n") {
		t.Fatalf("generated pv.yml missing version 1:\n%s", generated)
	}
	if !strings.Contains(generated, "php: 8.4\n") {
		t.Fatalf("generated pv.yml missing php declaration:\n%s", generated)
	}
	assertMissing(t, filepath.Join(fixture.Root, ".env"))

	envContents := "APP_NAME=Existing\nUSER_SECRET=keep\n"
	if err := fixture.WriteEnv(envContents); err != nil {
		t.Fatalf("write existing env: %v", err)
	}
	refusal := runner.Run(t.Context(), fixture.Root, "init")
	if refusal.ExitCode == 0 {
		t.Fatalf("pv init overwrote existing pv.yml without --force")
	}
	if !strings.Contains(refusal.Stderr, "pv.yml already exists") || !strings.Contains(refusal.Stderr, "--force") {
		t.Fatalf("pv init refusal was not actionable, stderr:\n%s", refusal.Stderr)
	}
	if got := readFile(t, pvYMLPath); got != generated {
		t.Fatalf("pv init refusal mutated pv.yml:\n%s", got)
	}
	if got := readFile(t, filepath.Join(fixture.Root, ".env")); got != envContents {
		t.Fatalf("pv init refusal mutated .env:\n%s", got)
	}

	custom := "version: 1\nphp: 8.4\nhosts:\n  - custom.test\nservices:\nsetup:\n"
	if err := os.WriteFile(pvYMLPath, []byte(custom), 0o644); err != nil {
		t.Fatalf("write custom pv.yml: %v", err)
	}
	forced := runner.Run(t.Context(), fixture.Root, "init", "--force")
	if forced.ExitCode != 0 {
		t.Fatalf("pv init --force exit = %d, stderr:\n%s", forced.ExitCode, forced.Stderr)
	}
	if got := readFile(t, pvYMLPath); got != generated {
		t.Fatalf("pv init --force did not restore deterministic pv.yml:\n%s", got)
	}
	if got := readFile(t, filepath.Join(fixture.Root, ".env")); got != envContents {
		t.Fatalf("pv init --force mutated .env:\n%s", got)
	}
}

func findRepoRoot(t *testing.T) string {
	t.Helper()

	dir, err := os.Getwd()
	if err != nil {
		t.Fatalf("get working directory: %v", err)
	}
	for {
		if _, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil {
			return dir
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			t.Fatal("could not find repository root")
		}
		dir = parent
	}
}

func readFile(t *testing.T, path string) string {
	t.Helper()

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	return string(data)
}

func assertMissing(t *testing.T, path string) {
	t.Helper()

	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("expected %s to be missing, got stat error %v", path, err)
	}
}
