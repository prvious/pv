package ui

import (
	"fmt"
	"os"
	"time"
)

const progressBarWidth = 40

// ProgressWriter wraps an io.Writer and displays a progress bar.
type ProgressWriter struct {
	total    int64
	written  int64
	label    string
	lastDraw time.Time
}

// NewProgressWriter creates a progress writer for tracking download progress.
// If total is 0, no progress bar is shown.
func NewProgressWriter(label string, total int64) *ProgressWriter {
	ensureCursorRestore()
	fmt.Fprint(os.Stderr, "\033[?25l")
	return &ProgressWriter{
		total: total,
		label: label,
	}
}

func (pw *ProgressWriter) Write(p []byte) (int, error) {
	n := len(p)
	pw.written += int64(n)

	// Throttle redraws to avoid flicker
	if time.Since(pw.lastDraw) > 50*time.Millisecond || pw.written >= pw.total {
		pw.draw()
		pw.lastDraw = time.Now()
	}
	return n, nil
}

func (pw *ProgressWriter) draw() {
	if pw.total <= 0 {
		return
	}

	percent := int(pw.written * 100 / pw.total)
	if percent > 100 {
		percent = 100
	}

	on := percent * progressBarWidth / 100
	off := progressBarWidth - on

	filled := ""
	for i := 0; i < on; i++ {
		filled += "■"
	}
	empty := ""
	for i := 0; i < off; i++ {
		empty += "･"
	}

	fmt.Fprintf(os.Stderr, "\r  %s %3d%%", Purple.Render(filled+empty), percent)
}

// Finish clears the progress line and restores cursor.
func (pw *ProgressWriter) Finish() {
	fmt.Fprintf(os.Stderr, "\r\033[2K\033[?25h")
}
