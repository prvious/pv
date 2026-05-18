package cli

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/control"
)

const Version = "dev"

var ErrUsage = errors.New("usage error")

func Run(args []string, stdout io.Writer, stderr io.Writer) error {
	if len(args) == 0 {
		writeHelp(stdout)
		return nil
	}

	switch args[0] {
	case "help", "--help", "-h":
		writeHelp(stdout)
		return nil
	case "mago:install":
		return runMagoInstall(args[1:], stderr)
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
  help      Show this help.
  mago:install <version>
  status    Show desired and observed control-plane status.
  version   Print the pv version.

The active rewrite command surface is intentionally minimal. See docs/rewrite/02-architecture.md.
`)
}

func runMagoInstall(args []string, stderr io.Writer) error {
	if len(args) != 1 {
		fmt.Fprintln(stderr, "usage: pv mago:install <version>")
		return fmt.Errorf("%w: invalid mago:install command", ErrUsage)
	}

	version := args[0]
	if err := control.ValidateVersion(version); err != nil {
		fmt.Fprintf(stderr, "pv: %v\n", err)
		return fmt.Errorf("%w: %v", ErrUsage, err)
	}

	store, err := defaultStore()
	if err != nil {
		return err
	}
	if err := store.PutDesired(context.Background(), control.DesiredResource{
		Resource: control.ResourceMago,
		Version:  version,
	}); err != nil {
		return err
	}

	fmt.Fprintf(stderr, "requested mago %s install\n", version)
	return nil
}

func runStatus(stderr io.Writer) error {
	store, err := defaultStore()
	if err != nil {
		return err
	}

	ctx := context.Background()
	desired, desiredOK, err := store.Desired(ctx, control.ResourceMago)
	if err != nil {
		return err
	}
	observed, observedOK, err := store.Observed(ctx, control.ResourceMago)
	if err != nil {
		return err
	}

	if !desiredOK {
		fmt.Fprintln(stderr, "desired: none")
		return nil
	}

	fmt.Fprintf(stderr, "desired: mago %s install\n", desired.Version)
	if !observedOK {
		fmt.Fprintln(stderr, "observed: mago pending")
		fmt.Fprintln(stderr, "next action: run reconciliation")
		return nil
	}

	fmt.Fprintf(stderr, "observed: mago %s %s\n", observed.DesiredVersion, observed.State)
	fmt.Fprintf(stderr, "last reconcile: %s\n", observed.LastReconcileTime)
	if observed.LastError != "" {
		fmt.Fprintf(stderr, "last error: %s\n", observed.LastError)
	}
	if observed.NextAction != "" {
		fmt.Fprintf(stderr, "next action: %s\n", observed.NextAction)
	}
	return nil
}

func defaultStore() (*control.FileStore, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, err
	}
	return control.NewFileStore(filepath.Join(home, ".pv", "state", "pv.json")), nil
}
