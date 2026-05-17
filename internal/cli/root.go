package cli

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/host"
	"github.com/prvious/pv/internal/installer"
	"github.com/prvious/pv/internal/project"
	"github.com/prvious/pv/internal/status"
)

const Version = "dev"

var ErrUsage = errors.New("usage error")

func Run(args []string, stdout io.Writer, stderr io.Writer) error {
	if len(args) == 0 {
		writeHelp(stdout)
		return nil
	}

	switch args[0] {
	case "artisan":
		return runHelper(args[1:], stderr, project.Artisan)
	case "composer:install":
		return runComposerInstall(args[1:], stderr)
	case "db":
		return runCheckedHelper(args[1:], stderr, project.DB)
	case "help", "--help", "-h":
		writeHelp(stdout)
		return nil
	case "init":
		return runInit(args[1:], stderr)
	case "link":
		return runLink(stderr)
	case "mago:install":
		return runSimpleInstall(args[1:], stderr, control.ResourceMago, "pv mago:install <version>")
	case "mail":
		return runCheckedHelper(args[1:], stderr, project.Mail)
	case "open":
		return runOpen(stderr)
	case "php:install":
		return runSimpleInstall(args[1:], stderr, control.ResourcePHP, "pv php:install <version>")
	case "s3":
		return runCheckedHelper(args[1:], stderr, project.S3)
	case "status":
		return runStatus(args[1:], stderr)
	case "version", "--version":
		fmt.Fprintf(stdout, "pv %s\n", Version)
		return nil
	default:
		fmt.Fprintf(stderr, "pv: unknown command %q\nRun 'pv help' for usage.\n", args[0])
		return fmt.Errorf("%w: unknown command %q", ErrUsage, args[0])
	}
}

func writeHelp(w io.Writer) {
	fmt.Fprint(w, `pv rewrite control plane

Usage:
  pv <command>

Commands:
  artisan <args...>
  composer:install <version> --php <version>
  db <args...>
  help      Show this help.
  init [--force]
  link
  mago:install <version>
  mail <args...>
  open
  php:install <version>
  s3 <args...>
  status    Show desired and observed control-plane status.
  version   Print the pv version.

The active rewrite command surface is intentionally minimal. See docs/rewrite/02-architecture.md.
`)
}

func runInit(args []string, stderr io.Writer) error {
	force := len(args) == 1 && args[0] == "--force"
	if len(args) > 0 && !force {
		fmt.Fprintln(stderr, "usage: pv init [--force]")
		return fmt.Errorf("%w: invalid init command", ErrUsage)
	}
	cwd, err := os.Getwd()
	if err != nil {
		return err
	}
	if !project.DetectLaravel(cwd) {
		err := errors.New("current directory is not a supported Laravel project")
		fmt.Fprintf(stderr, "pv: %v\n", err)
		return err
	}
	contract := project.DefaultLaravelContract(filepath.Base(cwd))
	if err := project.WriteContract(cwd, contract, force); err != nil {
		fmt.Fprintf(stderr, "pv: %v\n", err)
		return err
	}
	fmt.Fprintln(stderr, "created pv.yml")
	return nil
}

func runLink(stderr io.Writer) error {
	cwd, err := os.Getwd()
	if err != nil {
		return err
	}
	contract, err := project.LoadContract(cwd)
	if err != nil {
		return err
	}
	paths, err := host.NewPaths()
	if err != nil {
		return err
	}
	registry := project.Registry{Path: filepath.Join(paths.Root(), "state", "project.json")}
	ctx := context.Background()
	if err := installer.PersistThenSignal(ctx, func(ctx context.Context) error {
		if err := registry.Link(ctx, cwd, contract); err != nil {
			return err
		}
		if err := (project.EnvWriter{Path: filepath.Join(cwd, ".env")}).Apply(envFor(contract)); err != nil {
			return err
		}
		if err := ensureSetupRuntime(paths, contract); err != nil {
			return err
		}
		if err := project.RunSetup(ctx, cwd, filepath.Join(paths.BinDir()), contract.Setup, shellRunner{}); err != nil {
			if recordErr := recordSetupFailure(ctx, paths, registry, err); recordErr != nil {
				return errors.Join(err, recordErr)
			}
			return err
		}
		return nil
	}, func(context.Context) error {
		return writeReconcileSignal(paths, registry.Path)
	}); err != nil {
		fmt.Fprintf(stderr, "pv: %v\n", err)
		if nextAction := linkFailureNextAction(err); nextAction != "" {
			fmt.Fprintf(stderr, "next action: %s\n", nextAction)
		}
		return err
	}
	fmt.Fprintln(stderr, "linked project")
	return nil
}

func writeReconcileSignal(paths host.Paths, statePath string) error {
	if _, err := os.Stat(statePath); err != nil {
		return fmt.Errorf("signal daemon after state write: %w", err)
	}
	signalPath := filepath.Join(paths.Root(), "state", "reconcile.signal")
	data := fmt.Sprintf("project_state=%s\n", statePath)
	return os.WriteFile(signalPath, []byte(data), 0o600)
}

func ensureSetupRuntime(paths host.Paths, contract project.Contract) error {
	if len(contract.Setup) == 0 {
		return nil
	}
	_, err := os.Stat(filepath.Join(paths.BinDir(), "php"))
	if err == nil {
		return nil
	}
	if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return fmt.Errorf("PHP runtime %s is not installed: run pv php:install %s", contract.PHP, contract.PHP)
}

func recordSetupFailure(ctx context.Context, paths host.Paths, registry project.Registry, err error) error {
	setupErr, ok := errors.AsType[*project.SetupError](err)
	if !ok {
		return nil
	}
	state, ok, err := registry.Current(ctx)
	if err != nil {
		return err
	}
	if !ok {
		return nil
	}
	name := projectStatusName(state)
	failure := project.Failure{
		View:       string(status.ViewProject),
		Name:       name,
		Scenario:   "setup",
		Command:    setupErr.Command,
		Expected:   "setup commands complete",
		Actual:     setupErr.Err.Error(),
		LogPath:    filepath.Join(paths.Root(), "logs", "setup", name+".log"),
		NextAction: setupFailureNextAction,
	}
	if err := os.MkdirAll(filepath.Dir(failure.LogPath), 0o755); err != nil {
		return err
	}
	if err := os.WriteFile(failure.LogPath, []byte(failure.Actual+"\n"), 0o600); err != nil {
		return err
	}
	return registry.RecordFailure(ctx, failure)
}

const setupFailureNextAction = "fix the failing setup command and run pv link again"

func linkFailureNextAction(err error) string {
	if _, ok := errors.AsType[*project.SetupError](err); ok {
		return setupFailureNextAction
	}
	return ""
}

func runOpen(stderr io.Writer) error {
	cwd, err := os.Getwd()
	if err != nil {
		return err
	}
	contract, err := project.LoadContract(cwd)
	if err != nil {
		return err
	}
	fmt.Fprintf(stderr, "open https://%s\n", contract.Hosts[0])
	return nil
}

func runSimpleInstall(args []string, stderr io.Writer, resource string, usage string) error {
	if len(args) != 1 {
		fmt.Fprintf(stderr, "usage: %s\n", usage)
		return fmt.Errorf("%w: invalid %s:install command", ErrUsage, resource)
	}

	version := args[0]
	if err := validateVersionArg(stderr, version); err != nil {
		return err
	}
	if err := putDesired(control.DesiredResource{Resource: resource, Version: version}); err != nil {
		return err
	}

	fmt.Fprintf(stderr, "requested %s %s install\n", resource, version)
	return nil
}

func runComposerInstall(args []string, stderr io.Writer) error {
	if len(args) != 3 || args[1] != "--php" {
		fmt.Fprintln(stderr, "usage: pv composer:install <version> --php <version>")
		return fmt.Errorf("%w: invalid composer:install command", ErrUsage)
	}

	version := args[0]
	runtimeVersion := args[2]
	if err := validateVersionArg(stderr, version); err != nil {
		return err
	}
	if err := validateVersionArg(stderr, runtimeVersion); err != nil {
		return err
	}
	if err := putDesired(control.DesiredResource{
		Resource:       control.ResourceComposer,
		Version:        version,
		RuntimeVersion: runtimeVersion,
	}); err != nil {
		return err
	}

	fmt.Fprintf(stderr, "requested composer %s install with php %s\n", version, runtimeVersion)
	return nil
}

func runHelper(args []string, stderr io.Writer, helper func(project.Contract, ...string) []string) error {
	contract, err := loadCurrentContract()
	if err != nil {
		return err
	}
	fmt.Fprintf(stderr, "run %s\n", strings.Join(helper(contract, args...), " "))
	return nil
}

func runCheckedHelper(args []string, stderr io.Writer, helper func(project.Contract, ...string) ([]string, error)) error {
	contract, err := loadCurrentContract()
	if err != nil {
		return err
	}
	command, err := helper(contract, args...)
	if err != nil {
		return err
	}
	fmt.Fprintf(stderr, "run %s\n", strings.Join(command, " "))
	return nil
}

func loadCurrentContract() (project.Contract, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return project.Contract{}, err
	}
	paths, err := host.NewPaths()
	if err != nil {
		return project.Contract{}, err
	}
	registry := project.Registry{Path: projectStatePath(paths)}
	state, ok, err := registry.Current(context.Background())
	if err != nil {
		return project.Contract{}, err
	}
	if ok && pathContains(state.Path, cwd) {
		return state.Contract(), nil
	}
	return project.LoadContract(cwd)
}

func runStatus(args []string, stderr io.Writer) error {
	if len(args) > 1 {
		fmt.Fprintln(stderr, "usage: pv status [project|runtime|resource|gateway]")
		return fmt.Errorf("%w: invalid status command", ErrUsage)
	}
	var view status.View
	if len(args) == 1 {
		view = status.View(args[0])
	}
	paths, err := host.NewPaths()
	if err != nil {
		return err
	}
	ctx := context.Background()
	store := control.NewFileStore(paths.StateDBPath())
	entries, err := collectStatusEntries(ctx, paths, store)
	if err != nil {
		return err
	}
	rendered, err := status.Render(entries, view)
	if err != nil {
		return err
	}
	fmt.Fprint(stderr, rendered)
	return nil
}

func validateVersionArg(stderr io.Writer, version string) error {
	if err := control.ValidateVersion(version); err != nil {
		fmt.Fprintf(stderr, "pv: %v\n", err)
		return fmt.Errorf("%w: %v", ErrUsage, err)
	}
	return nil
}

func putDesired(desired control.DesiredResource) error {
	store, err := defaultStore()
	if err != nil {
		return err
	}
	return store.PutDesired(context.Background(), desired)
}

func defaultStore() (*control.FileStore, error) {
	paths, err := host.NewPaths()
	if err != nil {
		return nil, err
	}
	return control.NewFileStore(paths.StateDBPath()), nil
}

func envFor(contract project.Contract) map[string]string {
	values := map[string]string{
		"PV_PHP": contract.PHP,
	}
	for _, service := range contract.Services {
		switch service {
		case "mailpit":
			values["MAIL_MAILER"] = "smtp"
			values["MAIL_HOST"] = "127.0.0.1"
			values["MAIL_PORT"] = "1025"
		case "postgres":
			values["DB_CONNECTION"] = "pgsql"
			values["DB_HOST"] = "127.0.0.1"
			values["DB_PORT"] = "5432"
		case "mysql":
			values["DB_CONNECTION"] = "mysql"
			values["DB_HOST"] = "127.0.0.1"
			values["DB_PORT"] = "3306"
		case "redis":
			values["REDIS_HOST"] = "127.0.0.1"
			values["REDIS_PORT"] = "6379"
		case "rustfs":
			values["AWS_ENDPOINT_URL"] = "http://127.0.0.1:9000"
		}
	}
	return values
}

func pathContains(root string, path string) bool {
	rel, err := filepath.Rel(canonicalPath(root), canonicalPath(path))
	if err != nil {
		return false
	}
	return rel == "." || (rel != ".." && !strings.HasPrefix(rel, ".."+string(os.PathSeparator)))
}

func canonicalPath(path string) string {
	resolved, err := filepath.EvalSymlinks(path)
	if err == nil {
		return resolved
	}
	return filepath.Clean(path)
}

type shellRunner struct{}

func (shellRunner) Run(ctx context.Context, dir string, command string, env map[string]string) error {
	cmd := exec.CommandContext(ctx, "sh", "-c", command)
	cmd.Dir = dir
	cmd.Env = mergeEnv(os.Environ(), env)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return commandRunError{Err: err, Stderr: strings.TrimSpace(stderr.String())}
	}
	return nil
}

type commandRunError struct {
	Err    error
	Stderr string
}

func (e commandRunError) Error() string {
	if e.Stderr == "" {
		return e.Err.Error()
	}
	return e.Err.Error() + ": " + e.Stderr
}

func (e commandRunError) Unwrap() error {
	return e.Err
}

func mergeEnv(base []string, overrides map[string]string) []string {
	merged := append([]string(nil), base...)
	for key, value := range overrides {
		prefix := key + "="
		replaced := false
		for i, entry := range merged {
			if strings.HasPrefix(entry, prefix) {
				merged[i] = prefix + value
				replaced = true
				break
			}
		}
		if !replaced {
			merged = append(merged, prefix+value)
		}
	}
	return merged
}
