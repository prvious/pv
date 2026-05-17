package php

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/host"
	"github.com/prvious/pv/internal/installer"
)

type Installer interface {
	Install(context.Context, string) error
}

type Controller struct {
	Store     control.Store
	Installer Installer
	Clock     func() time.Time
}

func (c Controller) Reconcile(ctx context.Context) error {
	desired, ok, err := c.Store.Desired(ctx, control.ResourcePHP)
	if err != nil {
		return err
	}
	if !ok {
		return nil
	}

	reconciledAt := c.now().UTC().Format(time.RFC3339)
	if err := control.ValidateVersion(desired.Version); err != nil {
		return c.recordFailure(ctx, desired.Version, reconciledAt, err)
	}

	if err := c.Installer.Install(ctx, desired.Version); err != nil {
		return c.recordFailure(ctx, desired.Version, reconciledAt, err)
	}

	return c.Store.PutObserved(ctx, control.ObservedStatus{
		Resource:          control.ResourcePHP,
		DesiredVersion:    desired.Version,
		State:             control.StateReady,
		LastReconcileTime: reconciledAt,
	})
}

func (c Controller) recordFailure(ctx context.Context, version string, reconciledAt string, cause error) error {
	status := control.ObservedStatus{
		Resource:          control.ResourcePHP,
		DesiredVersion:    version,
		State:             control.StateFailed,
		LastReconcileTime: reconciledAt,
		LastError:         cause.Error(),
		NextAction:        "fix the PHP runtime install failure and run reconciliation again",
	}
	if err := c.Store.PutObserved(ctx, status); err != nil {
		return err
	}
	return cause
}

func (c Controller) now() time.Time {
	if c.Clock != nil {
		return c.Clock()
	}
	return time.Now()
}

type MarkerInstaller struct {
	root string
}

func NewMarkerInstaller(root string) MarkerInstaller {
	return MarkerInstaller{root: root}
}

func (i MarkerInstaller) Install(ctx context.Context, version string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := control.ValidateVersion(version); err != nil {
		return err
	}

	marker := i.markerPath(version)
	if err := os.MkdirAll(filepath.Dir(marker), 0o755); err != nil {
		return err
	}
	if err := os.WriteFile(marker, []byte(fmt.Sprintf("php %s\n", version)), 0o644); err != nil {
		return err
	}
	return i.writeShim(version)
}

func (i MarkerInstaller) Installed(version string) bool {
	_, err := os.Stat(i.markerPath(version))
	return err == nil
}

func (i MarkerInstaller) ShimExists() bool {
	paths, err := host.NewPathsFromRoot(i.root)
	if err != nil {
		return false
	}
	_, err = os.Stat(filepath.Join(paths.BinDir(), "php"))
	return err == nil
}

func (i MarkerInstaller) markerPath(version string) string {
	paths, err := host.NewPathsFromRoot(i.root)
	if err != nil {
		return filepath.Join(i.root, "runtimes", "php", version, "installed")
	}
	dir, err := paths.PHPRuntimeDir(version)
	if err != nil {
		return filepath.Join(i.root, "runtimes", "php", version, "installed")
	}
	return filepath.Join(dir, "installed")
}

func (i MarkerInstaller) writeShim(version string) error {
	paths, err := host.NewPathsFromRoot(i.root)
	if err != nil {
		return err
	}
	const shimTemplate = `#!/bin/sh
# php %s
exec "$(dirname "$0")/../runtimes/php/%s/php" "$@"
`
	return installer.WriteShimAtomic(filepath.Join(paths.BinDir(), "php"), []byte(fmt.Sprintf(shimTemplate, version, version)))
}
