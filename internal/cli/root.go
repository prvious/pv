package cli

import (
	"context"
	"errors"
	"fmt"
	"io"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/host"
)

const Version = "dev"

var ErrUsage = errors.New("usage error")

func Run(args []string, stdout io.Writer, stderr io.Writer) error {
	if len(args) == 0 {
		writeHelp(stdout)
		return nil
	}

	switch args[0] {
	case "composer:install":
		return runComposerInstall(args[1:], stderr)
	case "help", "--help", "-h":
		writeHelp(stdout)
		return nil
	case "mago:install":
		return runSimpleInstall(args[1:], stderr, control.ResourceMago, "pv mago:install <version>")
	case "php:install":
		return runSimpleInstall(args[1:], stderr, control.ResourcePHP, "pv php:install <version>")
	case "status":
		return runStatus(stderr)
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
  composer:install <version> --php <version>
  help      Show this help.
  mago:install <version>
  php:install <version>
  status    Show desired and observed control-plane status.
  version   Print the pv version.

The active rewrite command surface is intentionally minimal. See docs/rewrite/02-architecture.md.
`)
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

func runStatus(stderr io.Writer) error {
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
