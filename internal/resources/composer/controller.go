package composer

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/control"
)

type Runtime interface {
	Installed(string) bool
}

type Installer interface {
	Install(context.Context, InstallRequest) error
}

type InstallRequest struct {
	Version        string
	RuntimeVersion string
}

type Controller struct {
	Store     control.Store
	Installer Installer
	Runtime   Runtime
	Clock     func() time.Time
}

func (c Controller) Reconcile(ctx context.Context) error {
	desired, ok, err := c.Store.Desired(ctx, control.ResourceComposer)
	if err != nil {
		return err
	}
	if !ok {
		return nil
	}

	reconciledAt := c.now().UTC().Format(time.RFC3339)
	if err := validateDesired(desired); err != nil {
		return c.recordFailure(ctx, desired, control.StateFailed, reconciledAt, err, "fix the Composer desired state and run reconciliation again")
	}

	if !c.Runtime.Installed(desired.RuntimeVersion) {
		err := fmt.Errorf("PHP runtime %s is not installed", desired.RuntimeVersion)
		return c.recordFailure(ctx, desired, control.StateBlocked, reconciledAt, err, fmt.Sprintf("run pv php:install %s", desired.RuntimeVersion))
	}

	if err := c.Installer.Install(ctx, InstallRequest{
		Version:        desired.Version,
		RuntimeVersion: desired.RuntimeVersion,
	}); err != nil {
		return c.recordFailure(ctx, desired, control.StateFailed, reconciledAt, err, "fix the Composer install failure and run reconciliation again")
	}

	return c.Store.PutObserved(ctx, control.ObservedStatus{
		Resource:          control.ResourceComposer,
		DesiredVersion:    desired.Version,
		RuntimeVersion:    desired.RuntimeVersion,
		State:             control.StateReady,
		LastReconcileTime: reconciledAt,
	})
}

func validateDesired(desired control.DesiredResource) error {
	if err := control.ValidateVersion(desired.Version); err != nil {
		return err
	}
	if desired.RuntimeVersion == "" {
		return errors.New("composer runtime version is required")
	}
	return control.ValidateVersion(desired.RuntimeVersion)
}

func (c Controller) recordFailure(ctx context.Context, desired control.DesiredResource, state string, reconciledAt string, cause error, nextAction string) error {
	status := control.ObservedStatus{
		Resource:          control.ResourceComposer,
		DesiredVersion:    desired.Version,
		RuntimeVersion:    desired.RuntimeVersion,
		State:             state,
		LastReconcileTime: reconciledAt,
		LastError:         cause.Error(),
		NextAction:        nextAction,
	}
	if err := c.Store.PutObserved(ctx, status); err != nil {
		return err
	}
	if state == control.StateBlocked {
		return nil
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

func (i MarkerInstaller) Install(ctx context.Context, request InstallRequest) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := control.ValidateVersion(request.Version); err != nil {
		return err
	}
	if err := control.ValidateVersion(request.RuntimeVersion); err != nil {
		return err
	}

	marker := i.markerPath(request.Version)
	if err := os.MkdirAll(filepath.Dir(marker), 0o755); err != nil {
		return err
	}
	if err := os.WriteFile(marker, []byte(fmt.Sprintf("composer %s\n", request.Version)), 0o644); err != nil {
		return err
	}

	return i.writeShim(ctx, request)
}

func (i MarkerInstaller) Installed(version string) bool {
	_, err := os.Stat(i.markerPath(version))
	return err == nil
}

func (i MarkerInstaller) ShimExists() bool {
	_, err := os.Stat(filepath.Join(i.root, "bin", "composer"))
	return err == nil
}

func (i MarkerInstaller) markerPath(version string) string {
	return filepath.Join(i.root, "tools", "composer", version, "installed")
}

func (i MarkerInstaller) writeShim(ctx context.Context, request InstallRequest) error {
	if err := ctx.Err(); err != nil {
		return err
	}

	bin := filepath.Join(i.root, "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		return err
	}

	temp, err := os.CreateTemp(bin, ".composer-*")
	if err != nil {
		return err
	}
	tempPath := temp.Name()
	defer os.Remove(tempPath)

	const shimTemplate = `#!/bin/sh
# composer %s via php %s
exec "$(dirname "$0")/../runtimes/php/%s/php" "$(dirname "$0")/../tools/composer/%s/composer.phar" "$@"
`
	content := fmt.Sprintf(shimTemplate, request.Version, request.RuntimeVersion, request.RuntimeVersion, request.Version)
	if _, err := temp.WriteString(content); err != nil {
		temp.Close()
		return err
	}
	if err := temp.Chmod(0o755); err != nil {
		temp.Close()
		return err
	}
	if err := temp.Close(); err != nil {
		return err
	}
	return os.Rename(tempPath, filepath.Join(bin, "composer"))
}
