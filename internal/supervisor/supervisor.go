package supervisor

import (
	"context"
	"errors"
)

type ProcessDefinition struct {
	Name    string
	Version string
	Command string
	Args    []string
	Env     map[string]string
	LogPath string
}

type ProcessStatus struct {
	Name    string
	Running bool
	PID     int
	LogPath string
	Error   string
}

type Host interface {
	Start(context.Context, ProcessDefinition) (ProcessStatus, error)
	Stop(context.Context, string) error
	Check(context.Context, string) (ProcessStatus, error)
}

type Supervisor struct {
	Host Host
}

func (s Supervisor) Start(ctx context.Context, definition ProcessDefinition) (ProcessStatus, error) {
	if s.Host == nil {
		return ProcessStatus{}, errors.New("supervisor host is required")
	}
	if definition.Name == "" {
		return ProcessStatus{}, errors.New("process name is required")
	}
	return s.Host.Start(ctx, definition)
}

func (s Supervisor) Stop(ctx context.Context, name string) error {
	if s.Host == nil {
		return errors.New("supervisor host is required")
	}
	if name == "" {
		return errors.New("process name is required")
	}
	return s.Host.Stop(ctx, name)
}

func (s Supervisor) Check(ctx context.Context, name string) (ProcessStatus, error) {
	if s.Host == nil {
		return ProcessStatus{}, errors.New("supervisor host is required")
	}
	if name == "" {
		return ProcessStatus{}, errors.New("process name is required")
	}
	return s.Host.Check(ctx, name)
}
