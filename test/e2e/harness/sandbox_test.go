package harness

import (
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func TestNewSandboxIsolatesPvPathsUnderTempRoot(t *testing.T) {
	root := t.TempDir()

	sandbox, err := NewSandbox(root)
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}

	paths := []string{
		sandbox.HomeDir,
		sandbox.PVRoot,
		sandbox.StateDir,
		sandbox.CacheDir,
		sandbox.ConfigDir,
		sandbox.DataDir,
		sandbox.LogsDir,
		sandbox.ProjectRoot,
		sandbox.TempDir,
	}
	for _, path := range paths {
		assertWithin(t, root, path)
		if info, err := os.Stat(path); err != nil {
			t.Fatalf("expected sandbox path %s to exist: %v", path, err)
		} else if !info.IsDir() {
			t.Fatalf("expected sandbox path %s to be a directory", path)
		}
	}

	realHome, err := os.UserHomeDir()
	if err != nil {
		t.Fatalf("user home: %v", err)
	}
	if sandbox.PVRoot == filepath.Join(realHome, ".pv") {
		t.Fatalf("sandbox would use real pv root: %s", sandbox.PVRoot)
	}
}

func TestNewSandboxRefusesRealPvRoot(t *testing.T) {
	realHome, err := os.UserHomeDir()
	if err != nil {
		t.Fatalf("user home: %v", err)
	}

	_, err = newSandbox(sandboxConfig{
		RootDir: t.TempDir(),
		HomeDir: realHome,
	})
	if err == nil {
		t.Fatal("expected real home sandbox to be refused")
	}
	if !strings.Contains(err.Error(), "real ~/.pv") {
		t.Fatalf("expected real ~/.pv refusal, got %v", err)
	}
}

func TestSandboxPortReservationsAreDistinctAndReserved(t *testing.T) {
	sandbox, err := NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}
	defer func() {
		if err := sandbox.Cleanup(); err != nil {
			t.Fatalf("cleanup sandbox: %v", err)
		}
	}()

	first, err := sandbox.ReservePort()
	if err != nil {
		t.Fatalf("reserve first port: %v", err)
	}
	second, err := sandbox.ReservePort()
	if err != nil {
		t.Fatalf("reserve second port: %v", err)
	}
	if first.Port == second.Port {
		t.Fatalf("expected distinct ports, got %d", first.Port)
	}
	if want := deterministicPortBase(sandbox.RootDir); first.Port != want {
		t.Fatalf("first reserved port = %d, want deterministic base %d", first.Port, want)
	}

	listener, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", first.Port))
	if err == nil {
		_ = listener.Close()
		t.Fatalf("expected reserved port %d to reject another listener", first.Port)
	}
	if err := first.Close(); err != nil {
		t.Fatalf("close first port reservation: %v", err)
	}
	listener, err = net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", first.Port))
	if err != nil {
		t.Fatalf("expected released port %d to be reusable: %v", first.Port, err)
	}
	_ = listener.Close()
}

func TestCommandRunnerCapturesCommandResult(t *testing.T) {
	sandbox, err := NewSandbox(t.TempDir())
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}
	defer func() {
		if err := sandbox.Cleanup(); err != nil {
			t.Fatalf("cleanup sandbox: %v", err)
		}
	}()

	runner := CommandRunner{
		Executable: os.Args[0],
		Sandbox:    sandbox,
		ExtraEnv:   map[string]string{"PV_E2E_HELPER_PROCESS": "runner"},
	}
	result := runner.Run(t.Context(), sandbox.ProjectRoot, "-test.run=TestCommandRunnerHelper", "--", "arg-one")

	if result.ExitCode != 7 {
		t.Fatalf("exit code = %d, want 7", result.ExitCode)
	}
	if result.WorkingDirectory != sandbox.ProjectRoot {
		t.Fatalf("working directory = %s, want %s", result.WorkingDirectory, sandbox.ProjectRoot)
	}
	if !slices.Equal(result.Argv[1:], []string{"-test.run=TestCommandRunnerHelper", "--", "arg-one"}) {
		t.Fatalf("unexpected argv: %#v", result.Argv)
	}
	if !strings.Contains(result.Stdout, "helper stdout") {
		t.Fatalf("expected captured stdout, got %q", result.Stdout)
	}
	if !strings.Contains(result.Stderr, "helper stderr") {
		t.Fatalf("expected captured stderr, got %q", result.Stderr)
	}
	if result.EnvDiff["HOME"] != sandbox.HomeDir {
		t.Fatalf("HOME env diff = %q, want %q", result.EnvDiff["HOME"], sandbox.HomeDir)
	}
	if result.EnvDiff["PV_E2E_LOG_DIR"] != sandbox.LogsDir {
		t.Fatalf("PV_E2E_LOG_DIR env diff = %q, want %q", result.EnvDiff["PV_E2E_LOG_DIR"], sandbox.LogsDir)
	}
	if result.Elapsed <= 0 {
		t.Fatalf("expected elapsed time to be recorded, got %s", result.Elapsed)
	}
	wantLogPath := filepath.Join(sandbox.LogsDir, "helper.log")
	if !slices.Contains(result.LogPaths, wantLogPath) {
		t.Fatalf("expected log path %s in %#v", wantLogPath, result.LogPaths)
	}
}

func TestSandboxCleanupRemovesFilesAndTrackedProcesses(t *testing.T) {
	root := t.TempDir()
	sandbox, err := NewSandbox(root)
	if err != nil {
		t.Fatalf("new sandbox: %v", err)
	}
	if err := os.WriteFile(filepath.Join(sandbox.LogsDir, "owned.log"), []byte("owned"), 0o644); err != nil {
		t.Fatalf("write sandbox file: %v", err)
	}

	command := exec.Command(os.Args[0], "-test.run=TestCommandRunnerHelper")
	command.Env = append(os.Environ(), "PV_E2E_HELPER_PROCESS=sleep")
	if err := command.Start(); err != nil {
		t.Fatalf("start helper process: %v", err)
	}
	sandbox.TrackCommand(command)

	if err := sandbox.Cleanup(); err != nil {
		t.Fatalf("cleanup sandbox: %v", err)
	}
	if _, err := os.Stat(root); !os.IsNotExist(err) {
		t.Fatalf("expected sandbox root removal, got stat error %v", err)
	}
	if command.ProcessState == nil {
		t.Fatal("expected cleanup to wait for tracked process")
	}
}

func TestCommandRunnerHelper(t *testing.T) {
	switch os.Getenv("PV_E2E_HELPER_PROCESS") {
	case "":
		return
	case "sleep":
		select {}
	case "runner":
		cwd, err := os.Getwd()
		if err != nil {
			fmt.Fprintf(os.Stderr, "getwd: %v\n", err)
			os.Exit(2)
		}
		fmt.Fprintf(os.Stdout, "helper stdout cwd=%s args=%s\n", cwd, strings.Join(os.Args, " "))
		fmt.Fprintln(os.Stderr, "helper stderr")
		logDir := os.Getenv("PV_E2E_LOG_DIR")
		if err := os.WriteFile(filepath.Join(logDir, "helper.log"), []byte("helper log"), 0o644); err != nil {
			fmt.Fprintf(os.Stderr, "write helper log: %v\n", err)
			os.Exit(2)
		}
		os.Exit(7)
	default:
		t.Fatalf("unknown helper process mode %q", os.Getenv("PV_E2E_HELPER_PROCESS"))
	}
}

func assertWithin(t *testing.T, root string, path string) {
	t.Helper()

	rel, err := filepath.Rel(root, path)
	if err != nil {
		t.Fatalf("rel %s to %s: %v", path, root, err)
	}
	if rel == ".." || strings.HasPrefix(rel, ".."+string(os.PathSeparator)) {
		t.Fatalf("expected %s to be inside %s", path, root)
	}
}
