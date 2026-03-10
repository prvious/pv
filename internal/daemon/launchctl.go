package daemon

import (
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
)

// domainTarget returns the launchd domain target for the current user (e.g. "gui/501").
func domainTarget() string {
	return fmt.Sprintf("gui/%d", os.Getuid())
}

// serviceTarget returns the full service target (e.g. "gui/501/dev.prvious.pv").
func serviceTarget() string {
	return domainTarget() + "/" + Label
}

// Install writes the plist to ~/Library/LaunchAgents/.
func Install(cfg PlistConfig) error {
	return WritePlist(cfg)
}

// Uninstall removes the plist file from ~/Library/LaunchAgents/.
func Uninstall() error {
	return RemovePlist()
}

// Load tells launchd to load the pv service.
func Load() error {
	out, err := exec.Command("launchctl", "load", PlistPath()).CombinedOutput()
	if err != nil {
		output := strings.TrimSpace(string(out))
		if output != "" {
			return fmt.Errorf("cannot start pv service: %w (%s)", err, output)
		}
		return fmt.Errorf("cannot start pv service: %w", err)
	}
	return nil
}

// Unload tells launchd to unload the pv service.
func Unload() error {
	out, err := exec.Command("launchctl", "unload", PlistPath()).CombinedOutput()
	if err != nil {
		output := strings.TrimSpace(string(out))
		if output != "" {
			return fmt.Errorf("cannot stop pv service: %w (%s)", err, output)
		}
		return fmt.Errorf("cannot stop pv service: %w", err)
	}
	return nil
}

// Restart asks launchd to kill and re-launch the pv service via kickstart -k.
func Restart() error {
	out, err := exec.Command("launchctl", "kickstart", "-k", serviceTarget()).CombinedOutput()
	if err != nil {
		output := strings.TrimSpace(string(out))
		if output != "" {
			return fmt.Errorf("cannot restart pv service: %w (%s)", err, output)
		}
		return fmt.Errorf("cannot restart pv service: %w", err)
	}
	return nil
}

// IsLoaded returns true if the pv service is registered with launchd.
func IsLoaded() bool {
	err := exec.Command("launchctl", "list", Label).Run()
	return err == nil
}

// GetPID returns the PID of the running pv service, or 0 if not running.
func GetPID() (int, error) {
	out, err := exec.Command("launchctl", "list", Label).Output()
	if err != nil {
		return 0, fmt.Errorf("pv service is not running")
	}

	// launchctl list <label> outputs a header line then a data line:
	//   "PID" "Status" "Label"
	//   12345 0        dev.prvious.pv
	// If not running, PID column shows "-".
	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	if len(lines) < 2 {
		return 0, fmt.Errorf("pv service is not running")
	}

	fields := strings.Fields(lines[1])
	if len(fields) < 1 || fields[0] == "-" {
		return 0, fmt.Errorf("pv service is loaded but not running")
	}

	pid, err := strconv.Atoi(fields[0])
	if err != nil {
		return 0, fmt.Errorf("cannot parse PID from launchctl output: %w", err)
	}

	return pid, nil
}
