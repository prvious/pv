package mailpit

import (
	"context"
	"time"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/host"
	"github.com/prvious/pv/internal/supervisor"
)

type Supervisor interface {
	Start(context.Context, supervisor.ProcessDefinition) (supervisor.ProcessStatus, error)
}

type Controller struct {
	Store      control.Store
	Paths      host.Paths
	Supervisor Supervisor
	Clock      func() time.Time
}

func (c Controller) Resource() string {
	return control.ResourceMailpit
}

func (c Controller) Reconcile(ctx context.Context) error {
	desired, ok, err := c.Store.Desired(ctx, control.ResourceMailpit)
	if err != nil {
		return err
	}
	if !ok {
		return nil
	}

	definition, err := c.ProcessDefinition(desired.Version)
	if err != nil {
		return c.record(ctx, desired.Version, control.StateFailed, err, "")
	}
	status, err := c.Supervisor.Start(ctx, definition)
	if err != nil {
		return c.record(ctx, desired.Version, control.StateFailed, err, "inspect the Mailpit log and run reconciliation again")
	}
	state := control.StateStopped
	if status.Running {
		state = control.StateReady
	}
	return c.record(ctx, desired.Version, state, nil, "")
}

func (c Controller) ProcessDefinition(version string) (supervisor.ProcessDefinition, error) {
	bin, err := c.Paths.ServiceBinDir(control.ResourceMailpit, version)
	if err != nil {
		return supervisor.ProcessDefinition{}, err
	}
	logPath, err := c.Paths.LogPath(control.ResourceMailpit, version)
	if err != nil {
		return supervisor.ProcessDefinition{}, err
	}
	return supervisor.ProcessDefinition{
		Name:    control.ResourceMailpit,
		Version: version,
		Command: bin + "/mailpit",
		Args:    []string{"--smtp", "127.0.0.1:1025", "--listen", "127.0.0.1:8025"},
		Env:     Env(),
		LogPath: logPath,
	}, nil
}

func Env() map[string]string {
	return map[string]string{
		"MAIL_MAILER": "smtp",
		"MAIL_HOST":   "127.0.0.1",
		"MAIL_PORT":   "1025",
	}
}

func (c Controller) record(ctx context.Context, version string, state string, cause error, nextAction string) error {
	status := control.ObservedStatus{
		Resource:          control.ResourceMailpit,
		DesiredVersion:    version,
		State:             state,
		LastReconcileTime: c.now().UTC().Format(time.RFC3339),
		NextAction:        nextAction,
	}
	if cause != nil {
		status.LastError = cause.Error()
	}
	return c.Store.PutObserved(ctx, status)
}

func (c Controller) now() time.Time {
	if c.Clock != nil {
		return c.Clock()
	}
	return time.Now()
}
