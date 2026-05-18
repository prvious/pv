package supervisor

import (
	"context"
	"testing"
)

func TestSupervisorDelegatesWithoutResourceKnowledge(t *testing.T) {
	host := &fakeHost{}
	supervisor := Supervisor{Host: host}
	definition := ProcessDefinition{
		Name:    "worker",
		Version: "1.0.0",
		Command: "/bin/worker",
		LogPath: "/tmp/worker.log",
	}

	status, err := supervisor.Start(t.Context(), definition)

	if err != nil {
		t.Fatalf("Start returned error: %v", err)
	}
	if !status.Running || status.Name != "worker" {
		t.Fatalf("status = %#v", status)
	}
	if host.started.Command != "/bin/worker" {
		t.Fatalf("started definition = %#v", host.started)
	}
}

type fakeHost struct {
	started ProcessDefinition
}

func (h *fakeHost) Start(_ context.Context, definition ProcessDefinition) (ProcessStatus, error) {
	h.started = definition
	return ProcessStatus{Name: definition.Name, Running: true, PID: 42, LogPath: definition.LogPath}, nil
}

func (h *fakeHost) Stop(context.Context, string) error {
	return nil
}

func (h *fakeHost) Check(_ context.Context, name string) (ProcessStatus, error) {
	return ProcessStatus{Name: name, Running: true}, nil
}
