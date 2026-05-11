package steps

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/phpenv"
)

// ApplySetupStep runs the lines in pv.yml's setup: block via bash -c,
// in order, fail-fast on first non-zero exit. The pinned PHP bin dir
// is prepended to PATH so `php artisan ...` resolves to the project's
// version. Each line gets its own shell — variables don't persist
// across lines; users who need shared state join lines with `&&`
// inside a single entry. Stdout/stderr stream directly so long
// commands like `composer install` produce live output instead of
// buffering.
type ApplySetupStep struct{}

var _ automation.Step = (*ApplySetupStep)(nil)

func (s *ApplySetupStep) Label() string  { return "Run pv.yml setup commands" }
func (s *ApplySetupStep) Gate() string   { return "apply_setup" }
func (s *ApplySetupStep) Critical() bool { return true }

func (s *ApplySetupStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.ProjectConfig.HasSetup()
}

func (s *ApplySetupStep) Run(ctx *automation.Context) (string, error) {
	env := buildSetupEnv(ctx.PHPVersion)
	for i, line := range ctx.ProjectConfig.Setup {
		cmd := exec.Command("bash", "-c", line)
		cmd.Dir = ctx.ProjectPath
		cmd.Env = env
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return "", fmt.Errorf("setup[%d] %q: %w", i, line, err)
		}
	}
	return fmt.Sprintf("ran %d command(s)", len(ctx.ProjectConfig.Setup)), nil
}

// buildSetupEnv copies os.Environ() and prepends the pinned PHP's bin
// directory to PATH. If phpVersion is empty or the PHP path can't be
// resolved, the host PATH is returned unchanged.
func buildSetupEnv(phpVersion string) []string {
	env := os.Environ()
	if phpVersion == "" {
		return env
	}
	phpBin := phpenv.PHPPath(phpVersion)
	if phpBin == "" {
		return env
	}
	binDir := filepath.Dir(phpBin)
	for i, e := range env {
		if strings.HasPrefix(e, "PATH=") {
			env[i] = "PATH=" + binDir + ":" + strings.TrimPrefix(e, "PATH=")
			return env
		}
	}
	return append(env, "PATH="+binDir)
}
