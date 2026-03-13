package packages

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"

	"github.com/prvious/pv/internal/config"
)

// runComposer executes a composer command and returns its output.
// Replaceable in tests for isolation.
var runComposer = defaultRunComposer

func defaultRunComposer(ctx context.Context, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, config.ComposerPharPath(), args...)
	cmd.Env = composerEnv()
	return cmd.CombinedOutput()
}

// composerEnv builds the environment for composer commands, ensuring
// COMPOSER_HOME and COMPOSER_CACHE_DIR point to our managed directories
// and our bin dir is on PATH for PHP resolution.
func composerEnv() []string {
	env := os.Environ()
	env = replaceOrAppendEnv(env, "COMPOSER_HOME", config.ComposerDir())
	env = replaceOrAppendEnv(env, "COMPOSER_CACHE_DIR", config.ComposerCacheDir())
	env = replaceOrAppendEnv(env, "PATH", config.BinDir()+":"+os.Getenv("PATH"))
	return env
}

func replaceOrAppendEnv(env []string, key, val string) []string {
	prefix := key + "="
	for i, e := range env {
		if strings.HasPrefix(e, prefix) {
			env[i] = prefix + val
			return env
		}
	}
	return append(env, prefix+val)
}

func composerGlobalRequire(ctx context.Context, pkg Package) (string, error) {
	out, err := runComposer(ctx, "global", "require", pkg.Composer, "--no-interaction", "--no-ansi")
	if err != nil {
		return "", fmt.Errorf("composer global require %s: %w\nOutput: %s", pkg.Composer, err, strings.TrimSpace(string(out)))
	}
	return getComposerPackageVersion(ctx, pkg)
}

func composerGlobalUpdate(ctx context.Context, pkg Package) (string, error) {
	out, err := runComposer(ctx, "global", "update", pkg.Composer, "--no-interaction", "--no-ansi")
	if err != nil {
		return "", fmt.Errorf("composer global update %s: %w\nOutput: %s", pkg.Composer, err, strings.TrimSpace(string(out)))
	}
	return getComposerPackageVersion(ctx, pkg)
}

// getComposerPackageVersion returns the installed version of a composer package.
// Composer >=2.x returns "version" as a string; older versions return
// "versions" as an array where the active version is prefixed with "* ".
func getComposerPackageVersion(ctx context.Context, pkg Package) (string, error) {
	out, err := runComposer(ctx, "global", "show", pkg.Composer, "--format=json", "--no-ansi")
	if err != nil {
		return "", fmt.Errorf("composer show %s: %w\nOutput: %s", pkg.Composer, err, strings.TrimSpace(string(out)))
	}

	var info struct {
		Version  string   `json:"version"`
		Versions []string `json:"versions"`
	}
	if err := json.Unmarshal(out, &info); err != nil {
		return "", fmt.Errorf("parse composer show output: %w", err)
	}

	if info.Version != "" {
		return info.Version, nil
	}
	if len(info.Versions) > 0 {
		return strings.TrimPrefix(info.Versions[0], "* "), nil
	}
	return "", fmt.Errorf("no version found for %s", pkg.Composer)
}
