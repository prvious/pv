package mago

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/host"
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
	desired, ok, err := c.Store.Desired(ctx, control.ResourceMago)
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
		if recordErr := c.recordFailure(ctx, desired.Version, reconciledAt, err); recordErr != nil {
			return recordErr
		}
		return err
	}

	return c.Store.PutObserved(ctx, control.ObservedStatus{
		Resource:          control.ResourceMago,
		DesiredVersion:    desired.Version,
		State:             control.StateReady,
		LastReconcileTime: reconciledAt,
	})
}

func (c Controller) recordFailure(ctx context.Context, version string, reconciledAt string, cause error) error {
	status := control.ObservedStatus{
		Resource:          control.ResourceMago,
		DesiredVersion:    version,
		State:             control.StateFailed,
		LastReconcileTime: reconciledAt,
		LastError:         cause.Error(),
		NextAction:        "fix the Mago install failure and run reconciliation again",
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
	return os.WriteFile(marker, []byte(fmt.Sprintf("mago %s\n", version)), 0o644)
}

func (i MarkerInstaller) Installed(version string) bool {
	_, err := os.Stat(i.markerPath(version))
	return err == nil
}

func (i MarkerInstaller) markerPath(version string) string {
	paths, err := host.NewPathsFromRoot(i.root)
	if err != nil {
		return filepath.Join(i.root, "tools", "mago", version, "installed")
	}
	dir, err := paths.ToolDir(control.ResourceMago, version)
	if err != nil {
		return filepath.Join(i.root, "tools", "mago", version, "installed")
	}
	return filepath.Join(dir, "installed")
}
