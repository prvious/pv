package server

import (
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"

	"github.com/prvious/pv/internal/config"
)

// FrankenPHP manages a FrankenPHP child process.
type FrankenPHP struct {
	cmd     *exec.Cmd
	done    chan error
	logFile *os.File
}

// StartFrankenPHP spawns FrankenPHP, redirects output to the caddy log file,
// and waits for the admin API to become ready (up to 5s).
func StartFrankenPHP() (*FrankenPHP, error) {
	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	caddyfile := config.CaddyfilePath()

	logPath := config.CaddyLogPath()
	logFile, err := os.OpenFile(logPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return nil, fmt.Errorf("cannot open log file %s: %w", logPath, err)
	}

	cmd := exec.Command(frankenphp, "run", "--config", caddyfile, "--adapter", "caddyfile")
	cmd.Stdout = logFile
	cmd.Stderr = logFile

	if err := cmd.Start(); err != nil {
		logFile.Close()
		return nil, fmt.Errorf("cannot start FrankenPHP: %w", err)
	}

	fp := &FrankenPHP{
		cmd:     cmd,
		done:    make(chan error, 1),
		logFile: logFile,
	}
	go func() { fp.done <- cmd.Wait() }()

	// Wait for admin API to become ready.
	deadline := time.Now().Add(5 * time.Second)
	ready := false
	for time.Now().Before(deadline) {
		select {
		case err := <-fp.done:
			logFile.Close()
			return nil, fmt.Errorf("FrankenPHP exited during startup: %v", err)
		default:
		}

		resp, err := http.Get("http://localhost:2019/config/")
		if err == nil {
			resp.Body.Close()
			ready = true
			break
		}
		time.Sleep(200 * time.Millisecond)
	}

	if !ready {
		fp.Stop()
		return nil, fmt.Errorf("FrankenPHP admin API did not become ready within 5s")
	}

	return fp, nil
}

// Stop sends SIGTERM and waits up to 10s, then SIGKILL.
func (fp *FrankenPHP) Stop() error {
	if fp.cmd.Process == nil {
		return nil
	}
	defer fp.logFile.Close()

	// Send SIGTERM.
	fp.cmd.Process.Signal(syscall.SIGTERM)

	select {
	case <-fp.done:
		return nil
	case <-time.After(10 * time.Second):
		fp.cmd.Process.Kill()
		<-fp.done
		return nil
	}
}

// Done returns a channel that receives when the FrankenPHP process exits.
func (fp *FrankenPHP) Done() <-chan error {
	return fp.done
}

// Reload tells FrankenPHP to reload its configuration.
func Reload() error {
	frankenphp := filepath.Join(config.BinDir(), "frankenphp")
	caddyfile := config.CaddyfilePath()

	cmd := exec.Command(frankenphp, "reload", "--config", caddyfile, "--adapter", "caddyfile")
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("reload failed: %w: %s", err, string(output))
	}
	return nil
}
