package ui

import (
	"errors"
	"fmt"
	"os"

	"charm.land/lipgloss/v2"
)

// Colors matching install.sh: PURPLE=#b39ddb (ANSI 141), GREEN, RED, MUTED (dim).
var (
	Purple = lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(141))
	Green  = lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(2))
	Red    = lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(1))
	Muted  = lipgloss.NewStyle().Faint(true)
	Bold   = lipgloss.NewStyle().Bold(true)
)

// ErrAlreadyPrinted is returned when the error has already been displayed
// to the user via styled output. Callers should exit without printing again.
var ErrAlreadyPrinted = errors.New("")

// Header prints the pv version banner.
func Header(version string) {
	fmt.Fprintf(os.Stderr, "\n  %s %s\n\n",
		Purple.Bold(true).Render("pv"),
		Muted.Render("v"+version),
	)
}

// Success prints a green checkmark line.
func Success(text string) {
	fmt.Fprintf(os.Stderr, "  %s %s\n", Green.Render("✓"), text)
}

// Fail prints a red cross line.
func Fail(text string) {
	fmt.Fprintf(os.Stderr, "  %s %s\n", Red.Render("✗"), text)
}

// Subtle prints muted text.
func Subtle(text string) {
	fmt.Fprintf(os.Stderr, "  %s\n", Muted.Render(text))
}

// FailDetail prints indented detail under a failure.
func FailDetail(text string) {
	fmt.Fprintf(os.Stderr, "    %s\n", Muted.Render(text))
}

// Fatal prints an error and exits.
func Fatal(err error) {
	Fail(err.Error())
	os.Exit(1)
}
