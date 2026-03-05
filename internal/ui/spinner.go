package ui

import (
	"fmt"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"
)

var spinnerFrames = []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

type spinner struct {
	label string
	stop  chan struct{}
	done  chan struct{}
}

func newSpinner(label string) *spinner {
	return &spinner{
		label: label,
		stop:  make(chan struct{}),
		done:  make(chan struct{}),
	}
}

func (s *spinner) start() {
	// Hide cursor
	fmt.Fprint(os.Stderr, "\033[?25l")

	go func() {
		defer close(s.done)
		i := 0
		ticker := time.NewTicker(80 * time.Millisecond)
		defer ticker.Stop()

		for {
			frame := spinnerFrames[i%len(spinnerFrames)]
			fmt.Fprintf(os.Stderr, "\r  %s %s", Purple.Render(frame), Muted.Render(s.label))
			i++

			select {
			case <-s.stop:
				return
			case <-ticker.C:
			}
		}
	}()
}

func (s *spinner) finish() {
	close(s.stop)
	<-s.done
	// Clear the spinner line
	fmt.Fprintf(os.Stderr, "\r\033[2K")
	// Show cursor
	fmt.Fprint(os.Stderr, "\033[?25h")
}

// cursorGuard ensures cursor is restored on SIGINT.
var cursorGuard sync.Once

func ensureCursorRestore() {
	cursorGuard.Do(func() {
		c := make(chan os.Signal, 1)
		signal.Notify(c, os.Interrupt, syscall.SIGTERM)
		go func() {
			<-c
			fmt.Fprint(os.Stderr, "\033[?25h\n")
			os.Exit(1)
		}()
	})
}

// Step runs fn with a spinner, showing the label while working.
// On success, prints "✓ result". On failure, prints "✗ label" with error details.
func Step(label string, fn func() (string, error)) error {
	ensureCursorRestore()

	s := newSpinner(label)
	s.start()

	result, err := fn()
	s.finish()

	if err != nil {
		Fail(label)
		FailDetail(err.Error())
		return err
	}

	Success(result)
	return nil
}

// StepVerbose runs fn with verbose output (no spinner, direct prints allowed).
func StepVerbose(label string, fn func() (string, error)) error {
	fmt.Fprintf(os.Stderr, "  %s\n", label)

	result, err := fn()
	if err != nil {
		Fail(label)
		FailDetail(err.Error())
		return err
	}

	Success(result)
	return nil
}

// StepProgress runs fn with a progress bar for downloads.
// The fn receives a ProgressFunc that it should pass to download operations.
func StepProgress(label string, fn func(progress func(written, total int64)) (string, error)) error {
	ensureCursorRestore()

	var pw *ProgressWriter
	progressFn := func(written, total int64) {
		if pw == nil && total > 0 {
			pw = NewProgressWriter(label, total)
		}
		if pw != nil {
			pw.written = written
			now := time.Now()
			if now.Sub(pw.lastDraw) > 50*time.Millisecond || written >= total {
				pw.draw()
				pw.lastDraw = now
			}
		}
	}

	result, err := fn(progressFn)

	if pw != nil {
		pw.Finish()
	}

	if err != nil {
		Fail(label)
		FailDetail(err.Error())
		return err
	}

	Success(result)
	return nil
}
