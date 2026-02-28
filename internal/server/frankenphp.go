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
	version string // PHP version this instance serves ("" = main/global)
}

// StartFrankenPHP spawns the main FrankenPHP instance using the global binary.
// It uses the main Caddyfile and caddy.log.
func StartFrankenPHP() (*FrankenPHP, error) {
	return startFrankenPHPInstance(
		filepath.Join(config.BinDir(), "frankenphp"),
		config.CaddyfilePath(),
		config.CaddyLogPath(),
		"http://localhost:2019/config/",
		"",
	)
}

// StartVersionFrankenPHP spawns a secondary FrankenPHP instance for a specific
// PHP version, using the version-specific binary and Caddyfile.
func StartVersionFrankenPHP(version string) (*FrankenPHP, error) {
	fpPath := filepath.Join(config.PhpVersionDir(version), "frankenphp")
	caddyfile := config.VersionCaddyfilePath(version)
	logPath := config.CaddyLogPathForVersion(version)

	// Secondary instances have admin disabled, so no health check URL.
	return startFrankenPHPInstance(fpPath, caddyfile, logPath, "", version)
}

func startFrankenPHPInstance(fpPath, caddyfile, logPath, healthURL, version string) (*FrankenPHP, error) {
	logFile, err := os.OpenFile(logPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return nil, fmt.Errorf("cannot open log file %s: %w", logPath, err)
	}

	cmd := exec.Command(fpPath, "run", "--config", caddyfile, "--adapter", "caddyfile")
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
		version: version,
	}
	go func() {
		fp.done <- cmd.Wait()
		close(fp.done)
	}()

	// Wait for health check if URL provided (main process only).
	if healthURL != "" {
		deadline := time.Now().Add(5 * time.Second)
		ready := false
		for time.Now().Before(deadline) {
			select {
			case err := <-fp.done:
				logFile.Close()
				return nil, fmt.Errorf("FrankenPHP exited during startup: %v", err)
			default:
			}

			resp, err := http.Get(healthURL)
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
	} else {
		// For secondary instances, give a brief moment for startup.
		time.Sleep(500 * time.Millisecond)
		select {
		case err := <-fp.done:
			logFile.Close()
			return nil, fmt.Errorf("FrankenPHP (PHP %s) exited during startup: %v", version, err)
		default:
		}
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

// Version returns the PHP version this instance serves ("" for main).
func (fp *FrankenPHP) Version() string {
	return fp.version
}

// Reload tells the main FrankenPHP to reload its configuration.
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
