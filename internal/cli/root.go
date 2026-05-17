package cli

import (
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
		return errors.New("current directory is not a supported Laravel project")
	}
	contract := project.DefaultLaravelContract(filepath.Base(cwd))
	if err := project.WriteContract(cwd, contract, force); err != nil {
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
	if err := registry.Link(context.Background(), cwd, contract); err != nil {
		return err
	}
	if err := (project.EnvWriter{Path: filepath.Join(cwd, ".env")}).Apply(envFor(contract)); err != nil {
		return err
	}
	if err := project.RunSetup(context.Background(), cwd, filepath.Join(paths.BinDir()), contract.Setup, shellRunner{}); err != nil {
		return err
	}
	fmt.Fprintln(stderr, "linked project")
	return nil
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
	return project.LoadContract(cwd)
}

func runStatus(args []string, stderr io.Writer) error {
	if len(args) > 1 {
		fmt.Fprintln(stderr, "usage: pv status [project|runtime|resource|gateway]")
		return fmt.Errorf("%w: invalid status command", ErrUsage)
	}
	if len(args) == 1 {
		return runTargetedStatus(status.View(args[0]), stderr)
	}

	store, err := defaultStore()
	if err != nil {
		return err
	}

	ctx := context.Background()
	anyDesired := false
	for _, resource := range [...]string{control.ResourcePHP, control.ResourceComposer, control.ResourceMago} {
		desired, desiredOK, err := store.Desired(ctx, resource)
		if err != nil {
			return err
		}
		if !desiredOK {
			continue
		}
		anyDesired = true
		printDesired(stderr, desired)

		observed, observedOK, err := store.Observed(ctx, resource)
		if err != nil {
			return err
		}
		if !observedOK {
			fmt.Fprintf(stderr, "observed: %s pending\n", resource)
			fmt.Fprintln(stderr, "next action: run reconciliation")
			continue
		}
		printObserved(stderr, observed)
	}

	if !anyDesired {
		fmt.Fprintln(stderr, "desired: none")
	}
	return nil
}

func runTargetedStatus(view status.View, stderr io.Writer) error {
	rendered, err := status.Render(nil, view)
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

func printDesired(w io.Writer, desired control.DesiredResource) {
	if desired.RuntimeVersion != "" {
		fmt.Fprintf(w, "desired: %s %s install with php %s\n", desired.Resource, desired.Version, desired.RuntimeVersion)
		return
	}
	fmt.Fprintf(w, "desired: %s %s install\n", desired.Resource, desired.Version)
}

func printObserved(w io.Writer, observed control.ObservedStatus) {
	fmt.Fprintf(w, "observed: %s %s %s\n", observed.Resource, observed.DesiredVersion, observed.State)
	fmt.Fprintf(w, "last reconcile: %s\n", observed.LastReconcileTime)
	if observed.LastError != "" {
		fmt.Fprintf(w, "last error: %s\n", observed.LastError)
	}
	if observed.NextAction != "" {
		fmt.Fprintf(w, "next action: %s\n", observed.NextAction)
	}
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

type shellRunner struct{}

func (shellRunner) Run(ctx context.Context, dir string, command string, env map[string]string) error {
	cmd := exec.CommandContext(ctx, "sh", "-lc", command)
	cmd.Dir = dir
	cmd.Env = os.Environ()
	for key, value := range env {
		cmd.Env = append(cmd.Env, key+"="+value)
	}
	return cmd.Run()
}
