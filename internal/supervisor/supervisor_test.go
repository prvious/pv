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
