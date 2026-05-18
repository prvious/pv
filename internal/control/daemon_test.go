package control

import (
	"context"
	"errors"
	"testing"
)

func TestDaemonReconcilesControllersInOrder(t *testing.T) {
	var called []string
	daemon := Daemon{Controllers: []Controller{
		fakeController{resource: ResourcePHP, called: &called},
		fakeController{resource: ResourceComposer, called: &called},
	}}

	if err := daemon.Reconcile(t.Context()); err != nil {
		t.Fatalf("Reconcile returned error: %v", err)
	}
	want := []string{ResourcePHP, ResourceComposer}
	for i := range want {
		if called[i] != want[i] {
			t.Fatalf("called[%d] = %q, want %q", i, called[i], want[i])
		}
	}
}

func TestDaemonWrapsControllerFailures(t *testing.T) {
	cause := errors.New("install failed")
	daemon := Daemon{Controllers: []Controller{
		fakeController{resource: ResourcePHP, err: cause},
	}}

	err := daemon.Reconcile(context.Background())

	if !errors.Is(err, cause) {
		t.Fatalf("Reconcile error = %v, want %v", err, cause)
	}
}

type fakeController struct {
	resource string
	called   *[]string
	err      error
}

func (c fakeController) Resource() string {
	return c.resource
}

func (c fakeController) Reconcile(context.Context) error {
	if c.called != nil {
		*c.called = append(*c.called, c.resource)
	}
	return c.err
}
