package server

import (
	"fmt"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"

	"github.com/prvious/pv/internal/config"
)

// Start is the supervisor entry point. It writes a PID file, starts the DNS
// server and FrankenPHP, then blocks until an OS signal or child exit.
func Start(tld string) error {
	if err := config.EnsureDirs(); err != nil {
		return fmt.Errorf("cannot create directories: %w", err)
	}

	if err := writePID(); err != nil {
		return fmt.Errorf("cannot write PID file: %w", err)
	}
	defer removePID()

	// Start DNS server in a goroutine.
	dnsServer := NewDNSServer(tld)
	dnsErr := make(chan error, 1)
	go func() { dnsErr <- dnsServer.Start() }()
	defer dnsServer.Shutdown()

	fmt.Printf("DNS server listening on %s\n", dnsServer.Addr)

	// Start FrankenPHP.
	fp, err := StartFrankenPHP()
	if err != nil {
		return fmt.Errorf("cannot start FrankenPHP: %w", err)
	}
	defer fp.Stop()

	fmt.Println("FrankenPHP started")
	fmt.Printf("Serving .%s domains on https (port 443) and http (port 80)\n", tld)

	// Wait for signals or child exit.
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)

	select {
	case sig := <-sigCh:
		fmt.Printf("\nReceived %s, shutting down...\n", sig)
	case err := <-dnsErr:
		if err != nil {
			fmt.Printf("DNS server error: %v\n", err)
		}
	case err := <-fp.Done():
		if err != nil {
			fmt.Printf("FrankenPHP exited: %v\n", err)
		}
	}

	return nil
}

// IsRunning checks if a pv supervisor process is currently running.
func IsRunning() bool {
	pid, err := ReadPID()
	if err != nil {
		return false
	}
	proc, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	// Signal 0 checks if process exists without sending a signal.
	return proc.Signal(syscall.Signal(0)) == nil
}

// ReadPID reads the PID from the PID file.
func ReadPID() (int, error) {
	data, err := os.ReadFile(config.PidFilePath())
	if err != nil {
		return 0, err
	}
	return strconv.Atoi(strings.TrimSpace(string(data)))
}

func writePID() error {
	return os.WriteFile(config.PidFilePath(), []byte(strconv.Itoa(os.Getpid())), 0644)
}

func removePID() {
	os.Remove(config.PidFilePath())
}
