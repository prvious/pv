package harness

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

// Binary describes the active rewrite executable built for E2E scenarios.
type Binary struct {
	Path     string
	Evidence BinaryEvidence
}

// BinaryEvidence records the build command and output path used by the harness.
type BinaryEvidence struct {
	Argv             []string
	WorkingDirectory string
	OutputPath       string
	Stdout           string
	Stderr           string
	ExitCode         int
	Elapsed          time.Duration
}

// BuildError reports a failed active binary build with captured stderr.
type BuildError struct {
	Argv             []string
	WorkingDirectory string
	ExitCode         int
	Stderr           string
	Err              error
}

func (e *BuildError) Error() string {
	message := fmt.Sprintf("%s failed in %s with exit code %d", strings.Join(e.Argv, " "), e.WorkingDirectory, e.ExitCode)
	if stderr := strings.TrimSpace(e.Stderr); stderr != "" {
		return message + ": " + stderr
	}
	if e.Err != nil {
		return message + ": " + e.Err.Error()
	}
	return message
}

func (e *BuildError) Unwrap() error {
	return e.Err
}

// BuildActiveBinary compiles the active rewrite pv binary from repoRoot into outputDir.
func BuildActiveBinary(ctx context.Context, repoRoot string, outputDir string) (Binary, error) {
	repoRoot, err := filepath.Abs(repoRoot)
	if err != nil {
		return Binary{}, fmt.Errorf("resolve repository root: %w", err)
	}
	repoRoot = filepath.Clean(repoRoot)
	if isLegacyPrototypeRoot(repoRoot) {
		return Binary{}, fmt.Errorf("refusing to build legacy/prototype binary at %s", repoRoot)
	}

	outputDir, err = filepath.Abs(outputDir)
	if err != nil {
		return Binary{}, fmt.Errorf("resolve binary output directory: %w", err)
	}
	if err := os.MkdirAll(outputDir, 0o755); err != nil {
		return Binary{}, fmt.Errorf("create binary output directory: %w", err)
	}

	outputPath := filepath.Join(outputDir, binaryFilename())
	argv := []string{"go", "build", "-o", outputPath, "."}
	command := exec.CommandContext(ctx, argv[0], argv[1:]...)
	command.Dir = repoRoot

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	command.Stdout = &stdout
	command.Stderr = &stderr

	started := time.Now()
	err = command.Run()
	evidence := BinaryEvidence{
		Argv:             argv,
		WorkingDirectory: repoRoot,
		OutputPath:       outputPath,
		Stdout:           stdout.String(),
		Stderr:           stderr.String(),
		ExitCode:         exitCode(err),
		Elapsed:          time.Since(started),
	}
	result := Binary{Path: outputPath, Evidence: evidence}
	if err != nil {
		return result, &BuildError{
			Argv:             argv,
			WorkingDirectory: repoRoot,
			ExitCode:         evidence.ExitCode,
			Stderr:           evidence.Stderr,
			Err:              err,
		}
	}

	return result, nil
}

func binaryFilename() string {
	if runtime.GOOS == "windows" {
		return "pv.exe"
	}
	return "pv"
}

func exitCode(err error) int {
	if err == nil {
		return 0
	}
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		return exitErr.ExitCode()
	}
	return -1
}

func isLegacyPrototypeRoot(root string) bool {
	legacyRoot := filepath.Join("legacy", "prototype")
	return root == legacyRoot || strings.HasSuffix(root, string(os.PathSeparator)+legacyRoot)
}
