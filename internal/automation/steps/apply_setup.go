package steps

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/phpenv"
)

// ApplySetupStep runs the lines in pv.yml's setup: block via bash -c,
// in order, fail-fast on first non-zero exit. The pinned PHP binary's
// directory is prepended to PATH (when ctx.PHPVersion is set) so
// `php artisan ...` resolves to the project's version. Each line gets
// its own shell — variables don't persist across lines. Stdout/stderr
// stream directly so long commands like `composer install` produce
// live output instead of buffering.
type ApplySetupStep struct{}

var _ automation.Step = (*ApplySetupStep)(nil)

func (s *ApplySetupStep) Label() string  { return "Run pv.yml setup commands" }
func (s *ApplySetupStep) Gate() string   { return "apply_setup" }
func (s *ApplySetupStep) Critical() bool { return true }
func (s *ApplySetupStep) Verbose() bool  { return true }

func (s *ApplySetupStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.ProjectConfig.HasSetup()
}

func (s *ApplySetupStep) Run(ctx *automation.Context) (string, error) {
	env := buildSetupEnv(ctx.PHPVersion)
	for i, line := range ctx.ProjectConfig.Setup {
		cmd := exec.Command("bash", "-c", line)
		cmd.Dir = ctx.ProjectPath
		cmd.Env = env

		// Tee stderr to both the real stderr (for live output) and a
		// buffer (so we can include the tail in the error message).
		var stderrBuf bytes.Buffer
		cmd.Stdout = os.Stdout
		cmd.Stderr = io.MultiWriter(os.Stderr, &stderrBuf)

		if err := cmd.Run(); err != nil {
			exitCode := -1
			if exitErr, ok := err.(*exec.ExitError); ok {
				exitCode = exitErr.ExitCode()
			}
			stderrTail := tailLines(stderrBuf.String(), 10)
			return "", fmt.Errorf("setup[%d] %q exited %d: %w\n%s", i, line, exitCode, err, stderrTail)
		}
	}
	return fmt.Sprintf("ran %d command(s)", len(ctx.ProjectConfig.Setup)), nil
}

// tailLines returns the last n newline-separated lines of s, or the
// whole string if it has fewer than n lines.
func tailLines(s string, n int) string {
	if s == "" {
		return ""
	}
	lines := strings.Split(s, "\n")
	if len(lines) <= n {
		return s
	}
	return strings.Join(lines[len(lines)-n:], "\n")
}

// buildSetupEnv copies os.Environ() and prepends the pinned PHP's bin
// directory to PATH. If phpVersion is empty, the host PATH is returned
// unchanged.
func buildSetupEnv(phpVersion string) []string {
	env := os.Environ()
	if phpVersion == "" {
		return env
	}
	binDir := filepath.Dir(phpenv.PHPPath(phpVersion))
	for i, e := range env {
		if rest, ok := strings.CutPrefix(e, "PATH="); ok {
			env[i] = "PATH=" + binDir + ":" + rest
			return env
		}
	}
	return append(env, "PATH="+binDir)
}
