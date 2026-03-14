package ui

import (
	"errors"
	"fmt"
	"os"

	"charm.land/lipgloss/v2"
)

// Accent is the primary brand color (#00D4AA teal). Green, Red, Orange are semantic.
var (
	Accent = lipgloss.NewStyle().Foreground(lipgloss.Color("#00D4AA"))
	Green  = lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(2))
	Red    = lipgloss.NewStyle().Foreground(lipgloss.ANSIColor(1))
	Orange = lipgloss.NewStyle().Foreground(lipgloss.Color("#FF6B35"))
	Muted  = lipgloss.NewStyle().Faint(true)
	Bold   = lipgloss.NewStyle().Bold(true)
)

// ErrAlreadyPrinted is returned when the error has already been displayed
// to the user via styled output. Callers should exit without printing again.
var ErrAlreadyPrinted = errors.New("error already printed")

// Header prints the pv version banner.
func Header(version string) {
	fmt.Fprintf(os.Stderr, "\n  %s %s\n\n",
		Accent.Bold(true).Render("pv"),
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

// SectionHeader prints a bold section header with surrounding spacing.
func SectionHeader(text string) {
	fmt.Fprintf(os.Stderr, "\n  %s\n", Bold.Render(text))
}
