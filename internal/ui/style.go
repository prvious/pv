package ui

import (
	"errors"
	"fmt"
	"os"

	"charm.land/lipgloss/v2"
)

// Raw color values for use in contexts that need lipgloss.Color directly.
var (
	AccentColor  = lipgloss.Color("#2F80FF")
	WarnColor    = lipgloss.Color("#FFD60A")
	SuccessColor = lipgloss.ANSIColor(2)
	DangerColor  = lipgloss.ANSIColor(1)
)

// Styles using semantic design tokens — not color names.
var (
	Accent   = lipgloss.NewStyle().Foreground(AccentColor)
	Positive = lipgloss.NewStyle().Foreground(SuccessColor)
	Negative = lipgloss.NewStyle().Foreground(DangerColor)
	Caution  = lipgloss.NewStyle().Foreground(WarnColor)
	Muted    = lipgloss.NewStyle().Faint(true)
	Bold     = lipgloss.NewStyle().Bold(true)
)

// ErrAlreadyPrinted is returned when the error has already been displayed
// to the user via styled output. Callers should exit without printing again.
var ErrAlreadyPrinted = errors.New("error already printed")

// ErrUserCancelled is returned when the user intentionally cancels an
// interactive operation (e.g. Ctrl+C / Esc). Exits non-zero without message.
var ErrUserCancelled = errors.New("user cancelled")

// Header prints the pv version banner.
func Header(version string) {
	fmt.Fprintf(os.Stderr, "\n  %s %s\n\n",
		Accent.Bold(true).Render("pv"),
		Muted.Render("v"+version),
	)
}

// Success prints a green checkmark line.
func Success(text string) {
	fmt.Fprintf(os.Stderr, "  %s %s\n", Positive.Render("✓"), text)
}

// Fail prints a red cross line.
func Fail(text string) {
	fmt.Fprintf(os.Stderr, "  %s %s\n", Negative.Render("✗"), text)
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
