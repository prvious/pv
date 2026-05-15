package php

import (
	"context"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/prvious/pv/internal/control"
)

func TestControllerReconcilesDesiredPHPRuntime(t *testing.T) {
	ctx := t.Context()
	store := control.NewFileStore(filepath.Join(t.TempDir(), "pv.json"))
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource: control.ResourcePHP,
		Version:  "8.4",
	}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}

	installer := NewMarkerInstaller(t.TempDir())
	controller := Controller{
		Store:     store,
		Installer: installer,
		Clock:     fixedClock("2026-05-15T19:00:00Z"),
	}

	if err := controller.Reconcile(ctx); err != nil {
		t.Fatalf("Reconcile returned error: %v", err)
	}

	if !installer.Installed("8.4") {
		t.Fatal("PHP runtime marker was not installed")
	}
	observed, ok, err := store.Observed(ctx, control.ResourcePHP)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find PHP status")
	}
	want := control.ObservedStatus{
		Resource:          control.ResourcePHP,
		DesiredVersion:    "8.4",
		State:             control.StateReady,
		LastReconcileTime: "2026-05-15T19:00:00Z",
	}
	if observed != want {
		t.Fatalf("Observed = %#v, want %#v", observed, want)
	}
}

func TestControllerRecordsPHPRuntimeInstallFailure(t *testing.T) {
	ctx := t.Context()
	store := control.NewFileStore(filepath.Join(t.TempDir(), "pv.json"))
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource: control.ResourcePHP,
		Version:  "8.4",
	}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}

	installErr := errors.New("download unavailable")
	controller := Controller{
		Store:     store,
		Installer: failingInstaller{err: installErr},
		Clock:     fixedClock("2026-05-15T19:00:00Z"),
	}

	if err := controller.Reconcile(ctx); !errors.Is(err, installErr) {
		t.Fatalf("Reconcile error = %v, want %v", err, installErr)
	}

	observed, ok, err := store.Observed(ctx, control.ResourcePHP)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find PHP status")
	}
	if observed.State != control.StateFailed {
		t.Fatalf("State = %q, want failed", observed.State)
	}
	if observed.LastError != "download unavailable" {
		t.Fatalf("LastError = %q, want download unavailable", observed.LastError)
	}
	if observed.NextAction == "" {
		t.Fatal("NextAction is empty for failed PHP reconcile")
	}
}

type failingInstaller struct {
	err error
}

func (i failingInstaller) Install(context.Context, string) error {
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
