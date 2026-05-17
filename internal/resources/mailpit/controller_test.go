package mailpit

import (
	"context"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/host"
	"github.com/prvious/pv/internal/supervisor"
)

func TestControllerStartsMailpitThroughSupervisor(t *testing.T) {
	ctx := t.Context()
	root := filepath.Join(t.TempDir(), ".pv")
	paths, err := host.NewPathsFromRoot(root)
	if err != nil {
		t.Fatalf("NewPathsFromRoot returned error: %v", err)
	}
	store := control.NewFileStore(filepath.Join(root, "state", "pv.db"))
	if err := store.PutDesired(ctx, control.DesiredResource{Resource: control.ResourceMailpit, Version: "1.0.0"}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}
	supervisor := &fakeSupervisor{}
	controller := Controller{
		Store:      store,
		Paths:      paths,
		Supervisor: supervisor,
		Clock:      fixedClock("2026-05-15T20:00:00Z"),
	}

	if err := controller.Reconcile(ctx); err != nil {
		t.Fatalf("Reconcile returned error: %v", err)
	}
	if supervisor.started.Name != control.ResourceMailpit {
		t.Fatalf("started process = %#v", supervisor.started)
	}
	if supervisor.started.Env["MAIL_PORT"] != "1025" {
		t.Fatalf("Mailpit env = %#v", supervisor.started.Env)
	}
	observed, ok, err := store.Observed(ctx, control.ResourceMailpit)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok || observed.State != control.StateReady {
		t.Fatalf("observed = %#v, ok=%v", observed, ok)
	}
}

func TestControllerReturnsStartErrorAfterRecordingFailure(t *testing.T) {
	ctx := t.Context()
	root := filepath.Join(t.TempDir(), ".pv")
	paths, err := host.NewPathsFromRoot(root)
	if err != nil {
		t.Fatalf("NewPathsFromRoot returned error: %v", err)
	}
	store := control.NewFileStore(filepath.Join(root, "state", "pv.db"))
	if err := store.PutDesired(ctx, control.DesiredResource{Resource: control.ResourceMailpit, Version: "1.0.0"}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}
	startErr := errors.New("mailpit start failed")
	controller := Controller{
		Store:      store,
		Paths:      paths,
		Supervisor: &fakeSupervisor{err: startErr},
		Clock:      fixedClock("2026-05-15T20:00:00Z"),
	}

	err = controller.Reconcile(ctx)

	if !errors.Is(err, startErr) {
		t.Fatalf("Reconcile error = %v, want %v", err, startErr)
	}
	observed, ok, err := store.Observed(ctx, control.ResourceMailpit)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find mailpit status")
	}
	if observed.State != control.StateFailed {
		t.Fatalf("observed state = %q, want failed", observed.State)
	}
	if observed.LastError != startErr.Error() {
		t.Fatalf("last error = %q, want %q", observed.LastError, startErr.Error())
	}
}

type fakeSupervisor struct {
	started supervisor.ProcessDefinition
	err     error
}

func (s *fakeSupervisor) Start(_ context.Context, definition supervisor.ProcessDefinition) (supervisor.ProcessStatus, error) {
	s.started = definition
	if s.err != nil {
		return supervisor.ProcessStatus{}, s.err
	}
	return supervisor.ProcessStatus{Name: definition.Name, Running: true, PID: 42, LogPath: definition.LogPath}, nil
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
