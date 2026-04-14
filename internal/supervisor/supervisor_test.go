package supervisor

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// newTestProcess returns a Process that runs `sh -c <cmd>` and writes logs
// to a file inside t.TempDir(). Ready is a no-op (returns nil immediately).
func newTestProcess(t *testing.T, name, shellCmd string) Process {
	t.Helper()
	logPath := filepath.Join(t.TempDir(), name+".log")
	return Process{
		Name:    name,
		Binary:  "/bin/sh",
		Args:    []string{"-c", shellCmd},
		LogFile: logPath,
		Ready: func(ctx context.Context) error {
			return nil
		},
		ReadyTimeout: 2 * time.Second,
	}
}

func TestSupervisor_StartStop_Sleep(t *testing.T) {
	s := New()
	p := newTestProcess(t, "sleeper", "sleep 30")
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	if !s.IsRunning("sleeper") {
		t.Fatal("expected IsRunning=true after Start")
	}
	if s.Pid("sleeper") == 0 {
		t.Error("Pid should be non-zero after successful Start")
	}
	if err := s.Stop("sleeper", 2*time.Second); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if s.IsRunning("sleeper") {
		t.Error("expected IsRunning=false after Stop")
	}
}

func TestSupervisor_StopAll(t *testing.T) {
	s := New()
	for _, name := range []string{"a", "b", "c"} {
		if err := s.Start(context.Background(), newTestProcess(t, name, "sleep 30")); err != nil {
			t.Fatalf("Start %s: %v", name, err)
		}
	}
	if err := s.StopAll(2 * time.Second); err != nil {
		t.Fatalf("StopAll: %v", err)
	}
	for _, name := range []string{"a", "b", "c"} {
		if s.IsRunning(name) {
			t.Errorf("%s still running after StopAll", name)
		}
	}
}

func TestSupervisor_ReadyTimeout(t *testing.T) {
	s := New()
	p := newTestProcess(t, "never-ready", "sleep 30")
	p.Ready = func(ctx context.Context) error {
		return errors.New("not ready")
	}
	p.ReadyTimeout = 500 * time.Millisecond

	start := time.Now()
	err := s.Start(context.Background(), p)
	elapsed := time.Since(start)

	if err == nil {
		t.Fatal("expected ready-timeout error")
	}
	if elapsed > 2*time.Second {
		t.Errorf("Start took %v; expected close to ReadyTimeout", elapsed)
	}
	// The process should have been killed after the timeout.
	if s.IsRunning("never-ready") {
		t.Error("expected process to be stopped after ready timeout")
	}
}

func TestSupervisor_SupervisedNames(t *testing.T) {
	s := New()
	if len(s.SupervisedNames()) != 0 {
		t.Error("expected empty list on fresh Supervisor")
	}
	if err := s.Start(context.Background(), newTestProcess(t, "x", "sleep 30")); err != nil {
		t.Fatalf("Start: %v", err)
	}
	names := s.SupervisedNames()
	if len(names) != 1 || names[0] != "x" {
		t.Errorf("SupervisedNames = %v, want [x]", names)
	}
	if err := s.Stop("x", 2*time.Second); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if len(s.SupervisedNames()) != 0 {
		t.Error("expected empty list after Stop")
	}
}

func TestSupervisor_TCPReadyCheck(t *testing.T) {
	// Bind a TCP listener on a random port inside the test and use it as the
	// ready-check target to prove the dial-based ready logic works.
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	addr := ln.Addr().String()

	s := New()
	p := Process{
		Name:    "ready-tcp",
		Binary:  "/bin/sh",
		Args:    []string{"-c", "sleep 30"},
		LogFile: filepath.Join(t.TempDir(), "ready-tcp.log"),
		Ready: func(ctx context.Context) error {
			c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
			if err != nil {
				return err
			}
			c.Close()
			return nil
		},
		ReadyTimeout: 3 * time.Second,
	}
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	defer s.Stop("ready-tcp", 2*time.Second)
	if !s.IsRunning("ready-tcp") {
		t.Error("expected IsRunning=true after ready check passes")
	}
}

func TestSupervisor_RestartsCrashedProcess(t *testing.T) {
	s := New()
	// A process that writes its pid to a file and exits with non-zero.
	// After crash the supervisor should respawn it — we detect respawn by
	// observing a *different* pid appear in the file.
	dir := t.TempDir()
	pidFile := filepath.Join(dir, "pid")
	script := "echo $$ > " + pidFile + "; exit 1"
	p := newTestProcess(t, "crasher", script)
	p.Ready = func(ctx context.Context) error { return nil }
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	defer s.Stop("crasher", 2*time.Second)

	// Wait for the first pid, then for a different pid (proof of respawn).
	firstPid := waitForPidFile(t, pidFile, 3*time.Second, "")
	respawnDeadline := time.Now().Add(10 * time.Second) // 2s sleep between restarts
	for time.Now().Before(respawnDeadline) {
		nextPid := waitForPidFile(t, pidFile, 1*time.Second, firstPid)
		if nextPid != "" && nextPid != firstPid {
			return // success: respawn observed
		}
		time.Sleep(200 * time.Millisecond)
	}
	t.Fatalf("process was not respawned within 10s; first pid=%s", firstPid)
}

func TestSupervisor_GivesUpAfterBudget(t *testing.T) {
	if testing.Short() {
		t.Skip("slow: exercises 5×2s restart budget")
	}
	s := New()
	// Immediately-exiting process. Supervisor respawns, each crash records
	// a restart timestamp; the 5th crash triggers budget exhaustion.
	p := newTestProcess(t, "fast-crasher", "exit 1")
	p.Ready = func(ctx context.Context) error { return nil }
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	// The budget is 5 restarts within 60s with a 2s sleep between each.
	// After ~5 iterations (~10s) the supervisor should give up and delete
	// the process from its tracking map.
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		if !s.IsRunning("fast-crasher") && len(s.SupervisedNames()) == 0 {
			return // success
		}
		time.Sleep(250 * time.Millisecond)
	}
	t.Errorf("supervisor did not give up within 15s; SupervisedNames=%v", s.SupervisedNames())
}

// waitForPidFile polls the file until it contains a pid that differs from
// excluded (or any pid if excluded is ""). Returns empty string on timeout.
func waitForPidFile(t *testing.T, path string, timeout time.Duration, excluded string) string {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		data, err := os.ReadFile(path)
		if err == nil {
			pid := strings.TrimSpace(string(data))
			if pid != "" && pid != excluded {
				return pid
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	return ""
}

func TestSupervisor_LogFileIsWritten(t *testing.T) {
	s := New()
	p := newTestProcess(t, "logger", "echo hello-from-supervisor; sleep 30")
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	defer s.Stop("logger", 2*time.Second)

	// Poll briefly for the log file to contain the expected line.
	deadline := time.Now().Add(2 * time.Second)
	var data []byte
	for time.Now().Before(deadline) {
		f, err := os.Open(p.LogFile)
		if err == nil {
			data, _ = io.ReadAll(f)
			f.Close()
			if len(data) > 0 {
				break
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	if len(data) == 0 {
		t.Fatalf("log file %s was empty after 2s", p.LogFile)
	}
	if !strings.Contains(string(data), "hello-from-supervisor") {
		t.Errorf("log file missing expected output; got %q", string(data))
	}
}
