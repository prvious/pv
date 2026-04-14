// Package supervisor spawns and watches child binary processes for the pv
// daemon. Each Process is launched, watched for liveness, restarted on crash
// (up to a budget), and stopped cleanly on demand. The supervisor runs
// in-process inside the pv daemon — it is not a standalone long-running
// process of its own.
package supervisor

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"syscall"
	"time"
)

// Process describes one supervised binary.
type Process struct {
	Name       string
	Binary     string // absolute path to executable
	Args       []string
	Env        []string // appended to os.Environ()
	WorkingDir string
	LogFile    string // absolute path; stdout+stderr appended here

	// Ready returns nil when the process is serving requests.
	Ready        func(ctx context.Context) error
	ReadyTimeout time.Duration
}

// managed holds the supervisor's internal state for a single Process.
type managed struct {
	proc     Process
	cmd      *exec.Cmd
	cancel   context.CancelFunc // cancels the watcher goroutine
	stopped  bool               // set true when Stop was called explicitly
	restarts []time.Time        // rolling window of restart timestamps
	// done is closed by the watcher goroutine when it returns (after
	// cmd.Wait has finished). Stop awaits this instead of calling
	// cmd.Wait itself — exec.Cmd.Wait is not safe for concurrent callers.
	done chan struct{}
}

// Supervisor manages a set of child processes.
type Supervisor struct {
	mu        sync.Mutex
	processes map[string]*managed
}

// New constructs an empty supervisor.
func New() *Supervisor {
	return &Supervisor{processes: map[string]*managed{}}
}

// Start spawns p, waits for p.Ready to succeed, and returns.
// The process continues in the background; crashes are restarted
// according to the crash-budget policy.
func (s *Supervisor) Start(ctx context.Context, p Process) error {
	if p.Name == "" {
		return errors.New("supervisor: Process.Name is required")
	}

	// Pre-flight: ensure log file's parent directory exists.
	if p.LogFile != "" {
		if err := os.MkdirAll(filepath.Dir(p.LogFile), 0o755); err != nil {
			return fmt.Errorf("supervisor: create log dir: %w", err)
		}
	}

	s.mu.Lock()
	if _, exists := s.processes[p.Name]; exists {
		s.mu.Unlock()
		return fmt.Errorf("supervisor: %q is already supervised", p.Name)
	}
	s.mu.Unlock()

	m, err := s.spawn(p)
	if err != nil {
		return err
	}

	s.mu.Lock()
	s.processes[p.Name] = m
	s.mu.Unlock()

	// Start the watch goroutine so a crash is observed immediately.
	watchCtx, cancel := context.WithCancel(context.Background())
	m.cancel = cancel
	go s.watch(watchCtx, p.Name)

	// Ready-wait (blocks Start).
	if p.Ready != nil {
		if err := s.waitReady(ctx, p); err != nil {
			_ = s.Stop(p.Name, 5*time.Second)
			return fmt.Errorf("supervisor: %s not ready: %w", p.Name, err)
		}
	}
	return nil
}

// spawn opens the log file, builds the command, and starts it.
func (s *Supervisor) spawn(p Process) (*managed, error) {
	var logFile *os.File
	if p.LogFile != "" {
		f, err := os.OpenFile(p.LogFile, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
		if err != nil {
			return nil, fmt.Errorf("supervisor: open log: %w", err)
		}
		logFile = f
	}

	cmd := exec.Command(p.Binary, p.Args...)
	cmd.Env = append(os.Environ(), p.Env...)
	cmd.Dir = p.WorkingDir
	if logFile != nil {
		cmd.Stdout = logFile
		cmd.Stderr = logFile
	}

	if err := cmd.Start(); err != nil {
		if logFile != nil {
			_ = logFile.Close()
		}
		return nil, fmt.Errorf("supervisor: spawn %s: %w", p.Name, err)
	}
	return &managed{proc: p, cmd: cmd, done: make(chan struct{})}, nil
}

// waitReady polls p.Ready every 250ms until success or timeout.
func (s *Supervisor) waitReady(ctx context.Context, p Process) error {
	deadline := time.Now().Add(p.ReadyTimeout)
	if p.ReadyTimeout == 0 {
		deadline = time.Now().Add(30 * time.Second)
	}
	for {
		if err := p.Ready(ctx); err == nil {
			return nil
		}
		if time.Now().After(deadline) {
			return errors.New("ready-check timed out")
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(250 * time.Millisecond):
		}
	}
}

// watch blocks on cmd.Wait and handles crash restarts. It is the sole caller
// of cmd.Wait for a given cmd — Stop does not call Wait directly (exec.Cmd
// does not support concurrent Wait callers); Stop awaits watch's exit via the
// managed.done channel instead.
func (s *Supervisor) watch(ctx context.Context, name string) {
	// Capture the initial done channel so we can close it reliably even if
	// the managed record is replaced via respawn — callers holding a
	// reference to the original managed.done need to be unblocked when the
	// original process exits.
	s.mu.Lock()
	initial, ok := s.processes[name]
	s.mu.Unlock()
	if !ok {
		return
	}
	currentDone := initial.done

	for {
		s.mu.Lock()
		m, ok := s.processes[name]
		s.mu.Unlock()
		if !ok {
			close(currentDone)
			return
		}

		waitErr := m.cmd.Wait()

		s.mu.Lock()
		if m.stopped {
			delete(s.processes, name)
			s.mu.Unlock()
			close(currentDone)
			return
		}

		// Crash recovery: enforce the restart budget (5 within 60s).
		now := time.Now()
		cutoff := now.Add(-60 * time.Second)
		filtered := m.restarts[:0]
		for _, t := range m.restarts {
			if t.After(cutoff) {
				filtered = append(filtered, t)
			}
		}
		m.restarts = filtered
		if len(m.restarts) >= 5 {
			fmt.Fprintf(os.Stderr, "supervisor: %s exceeded restart budget (5/60s); giving up (last error: %v)\n", name, waitErr)
			delete(s.processes, name)
			s.mu.Unlock()
			close(currentDone)
			return
		}
		m.restarts = append(m.restarts, now)
		s.mu.Unlock()

		// Pause briefly and respawn — no ready-wait on recovery.
		select {
		case <-ctx.Done():
			close(currentDone)
			return
		case <-time.After(2 * time.Second):
		}
		newM, err := s.spawn(m.proc)
		if err != nil {
			fmt.Fprintf(os.Stderr, "supervisor: %s respawn failed: %v\n", name, err)
			s.mu.Lock()
			delete(s.processes, name)
			s.mu.Unlock()
			close(currentDone)
			return
		}
		s.mu.Lock()
		// Stop may have been called between the spawn above (outside the
		// lock) and re-taking the lock here. If so, the freshly spawned
		// process is unwanted — kill it, drop it on the floor, and release
		// Stop's waiter via close(currentDone). Without this check, newM is
		// installed into the map just as Stop deletes the entry, orphaning
		// the live child with no supervisor and no kill path.
		if m.stopped {
			s.mu.Unlock()
			_ = newM.cmd.Process.Kill()
			_ = newM.cmd.Wait() // reap the zombie we just killed
			close(currentDone)
			return
		}
		newM.stopped = m.stopped
		newM.cancel = m.cancel
		newM.restarts = m.restarts
		s.processes[name] = newM
		s.mu.Unlock()
		// Signal that the original cmd.Wait has completed; Stop callers
		// holding the old done channel can proceed.
		close(currentDone)
		currentDone = newM.done
	}
}

// Stop sends SIGTERM, waits up to timeout, then SIGKILL.
// After Stop returns, IsRunning(name) is false. Stop awaits the watcher
// goroutine's exit (via managed.done) rather than calling cmd.Wait itself —
// exec.Cmd.Wait is not safe to call from multiple goroutines.
func (s *Supervisor) Stop(name string, timeout time.Duration) error {
	s.mu.Lock()
	m, ok := s.processes[name]
	if !ok {
		s.mu.Unlock()
		return nil
	}
	m.stopped = true
	if m.cancel != nil {
		m.cancel()
	}
	pid := m.cmd.Process.Pid
	cmd := m.cmd
	done := m.done
	s.mu.Unlock()

	if err := cmd.Process.Signal(syscall.SIGTERM); err != nil && !errors.Is(err, os.ErrProcessDone) {
		return fmt.Errorf("supervisor: SIGTERM %s (pid %d): %w", name, pid, err)
	}

	select {
	case <-done:
	case <-time.After(timeout):
		_ = cmd.Process.Kill()
		<-done
	}

	s.mu.Lock()
	delete(s.processes, name)
	s.mu.Unlock()
	return nil
}

// StopAll stops every supervised process in parallel.
// timeout is per-process, not total.
func (s *Supervisor) StopAll(timeout time.Duration) error {
	s.mu.Lock()
	names := make([]string, 0, len(s.processes))
	for n := range s.processes {
		names = append(names, n)
	}
	s.mu.Unlock()

	var wg sync.WaitGroup
	for _, n := range names {
		wg.Add(1)
		go func(name string) {
			defer wg.Done()
			_ = s.Stop(name, timeout)
		}(n)
	}
	wg.Wait()
	return nil
}

// IsRunning reports whether name is a supervised process with a live PID.
func (s *Supervisor) IsRunning(name string) bool {
	s.mu.Lock()
	m, ok := s.processes[name]
	s.mu.Unlock()
	if !ok {
		return false
	}
	// Signal 0 checks that the process exists without delivering a signal.
	return m.cmd.Process.Signal(syscall.Signal(0)) == nil
}

// Pid returns the current pid of name, or 0 if not supervised.
func (s *Supervisor) Pid(name string) int {
	s.mu.Lock()
	defer s.mu.Unlock()
	if m, ok := s.processes[name]; ok {
		return m.cmd.Process.Pid
	}
	return 0
}

// SupervisedNames returns the set of currently-supervised process names.
func (s *Supervisor) SupervisedNames() []string {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]string, 0, len(s.processes))
	for n := range s.processes {
		out = append(out, n)
	}
	return out
}
