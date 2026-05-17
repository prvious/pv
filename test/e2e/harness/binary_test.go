package harness

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestBuildActiveBinaryBuildsRootPvIntoTempPath(t *testing.T) {
	repoRoot := findRepoRoot(t)
	outputDir := filepath.Join(t.TempDir(), "bin")

	result, err := BuildActiveBinary(t.Context(), repoRoot, outputDir)
	if err != nil {
		t.Fatalf("build active binary: %v", err)
	}

	if result.Path == "" {
		t.Fatal("expected built binary path")
	}
	if !strings.HasPrefix(result.Path, outputDir+string(os.PathSeparator)) {
		t.Fatalf("expected binary under %s, got %s", outputDir, result.Path)
	}
	info, err := os.Stat(result.Path)
	if err != nil {
		t.Fatalf("stat built binary: %v", err)
	}
	if info.IsDir() {
		t.Fatalf("expected file, got directory: %s", result.Path)
	}
	if runtime.GOOS != "windows" && info.Mode()&0o111 == 0 {
		t.Fatalf("expected built binary to be executable, got mode %s", info.Mode())
	}
	if result.Evidence.OutputPath != result.Path {
		t.Fatalf("expected evidence output path %s, got %s", result.Path, result.Evidence.OutputPath)
	}
	if result.Evidence.WorkingDirectory != repoRoot {
		t.Fatalf("expected evidence working directory %s, got %s", repoRoot, result.Evidence.WorkingDirectory)
	}
	if got := strings.Join(result.Evidence.Argv, " "); !strings.Contains(got, "go build") {
		t.Fatalf("expected evidence command to include go build, got %q", got)
	}

	command := exec.Command(result.Path, "version")
	stdout, err := command.Output()
	if err != nil {
		t.Fatalf("run built binary version: %v", err)
	}
	if got, want := strings.TrimSpace(string(stdout)), "pv dev"; got != want {
		t.Fatalf("built binary version output = %q, want %q", got, want)
	}
}

func TestBuildActiveBinaryRefusesLegacyPrototypeRoot(t *testing.T) {
	repoRoot := findRepoRoot(t)
	legacyRoot := filepath.Join(repoRoot, "legacy", "prototype")

	result, err := BuildActiveBinary(t.Context(), legacyRoot, t.TempDir())
	if err == nil {
		t.Fatal("expected legacy prototype root to be refused")
	}
	if !strings.Contains(err.Error(), "legacy/prototype") {
		t.Fatalf("expected legacy/prototype refusal, got %v", err)
	}
	if result.Path != "" {
		t.Fatalf("expected no binary path, got %s", result.Path)
	}
}

func TestBuildActiveBinaryReportsBuildFailureEvidence(t *testing.T) {
	repoRoot := t.TempDir()
	writeFile(t, repoRoot, "go.mod", "module example.invalid/broken\n\ngo 1.26.3\n")
	writeFile(t, repoRoot, "main.go", "package main\nfunc main() {\n")

	result, err := BuildActiveBinary(t.Context(), repoRoot, t.TempDir())
	if err == nil {
		t.Fatal("expected build failure")
	}

	var buildErr *BuildError
	if !errors.As(err, &buildErr) {
		t.Fatalf("expected BuildError, got %T: %v", err, err)
	}
	if buildErr.ExitCode == 0 {
		t.Fatalf("expected non-zero exit code, got %d", buildErr.ExitCode)
	}
	if buildErr.Stderr == "" {
		t.Fatal("expected captured stderr")
	}
	if !strings.Contains(err.Error(), "go build") {
		t.Fatalf("expected error to include command, got %v", err)
	}
	if !strings.Contains(err.Error(), repoRoot) {
		t.Fatalf("expected error to include working directory %s, got %v", repoRoot, err)
	}
	if result.Evidence.ExitCode == 0 {
		t.Fatalf("expected evidence exit code to be non-zero, got %d", result.Evidence.ExitCode)
	}
	if result.Evidence.Stderr == "" {
		t.Fatal("expected evidence stderr")
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

func writeFile(t *testing.T, root string, name string, contents string) {
	t.Helper()

	path := filepath.Join(root, name)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("create fixture directory: %v", err)
	}
	if err := os.WriteFile(path, []byte(contents), 0o644); err != nil {
		t.Fatalf("write fixture file %s: %v", path, err)
	}
}
