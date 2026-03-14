package ca

import (
	"fmt"
	"os"
	"os/exec"

	"github.com/prvious/pv/internal/ui"
)

func acquireSudo() error {
	ui.Subtle("pv needs sudo to update the system keychain.")
	cmd := exec.Command("sudo", "-v")
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stderr
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("sudo authentication failed: %w", err)
	}
	fmt.Fprintln(os.Stderr)
	return nil
}
