package mago

import (
	"context"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/prvious/pv/internal/control"
)

var _ control.Controller = Controller{}

func TestControllerResource(t *testing.T) {
	if got := (Controller{}).Resource(); got != control.ResourceMago {
		t.Fatalf("Resource = %q, want %q", got, control.ResourceMago)
	}
}

func TestControllerReconcilesDesiredMagoInstallToObservedReady(t *testing.T) {
	ctx := context.Background()
	store := control.NewFileStore(filepath.Join(t.TempDir(), "pv.json"))
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource: control.ResourceMago,
		Version:  "1.2.3",
	}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}

	installer := &recordingInstaller{}
	controller := Controller{
		Store:     store,
		Installer: installer,
		Clock:     fixedClock("2026-05-15T18:00:00Z"),
	}

	if err := controller.Reconcile(ctx); err != nil {
		t.Fatalf("Reconcile returned error: %v", err)
	}

	if installer.version != "1.2.3" {
		t.Fatalf("installer version = %q, want 1.2.3", installer.version)
	}

	observed, ok, err := store.Observed(ctx, control.ResourceMago)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find mago status")
	}
	want := control.ObservedStatus{
		Resource:          control.ResourceMago,
		DesiredVersion:    "1.2.3",
		State:             control.StateReady,
		LastReconcileTime: "2026-05-15T18:00:00Z",
		NextAction:        "",
		LastError:         "",
	}
	if observed != want {
		t.Fatalf("Observed = %#v, want %#v", observed, want)
	}
}

func TestControllerRecordsFailedObservedStatus(t *testing.T) {
	ctx := context.Background()
	store := control.NewFileStore(filepath.Join(t.TempDir(), "pv.json"))
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource: control.ResourceMago,
		Version:  "1.2.3",
	}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}

	installErr := errors.New("disk full")
	controller := Controller{
		Store:     store,
		Installer: &recordingInstaller{err: installErr},
		Clock:     fixedClock("2026-05-15T18:00:00Z"),
	}

	if err := controller.Reconcile(ctx); !errors.Is(err, installErr) {
		t.Fatalf("Reconcile error = %v, want %v", err, installErr)
	}

	observed, ok, err := store.Observed(ctx, control.ResourceMago)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find mago status")
	}
	if observed.State != control.StateFailed {
		t.Fatalf("State = %q, want %q", observed.State, control.StateFailed)
	}
	if observed.LastError != "disk full" {
		t.Fatalf("LastError = %q, want disk full", observed.LastError)
	}
	if observed.NextAction == "" {
		t.Fatal("NextAction is empty for failed reconcile")
	}
}

func TestMarkerInstallerCreatesVersionMarker(t *testing.T) {
	ctx := context.Background()
	root := t.TempDir()
	installer := NewMarkerInstaller(root)

	if err := installer.Install(ctx, "1.2.3"); err != nil {
		t.Fatalf("Install returned error: %v", err)
	}

	if !installer.Installed("1.2.3") {
		t.Fatal("Installed returned false after Install")
	}
}

type recordingInstaller struct {
	version string
	err     error
}

func (i *recordingInstaller) Install(_ context.Context, version string) error {
	i.version = version
	return i.err
}

func fixedClock(value string) func() time.Time {
	parsed, err := time.Parse(time.RFC3339, value)
	if err != nil {
		panic(err)
	}
	return func() time.Time {
		return parsed
	}
}
