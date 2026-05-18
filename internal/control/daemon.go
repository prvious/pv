package control

import (
	"context"
	"errors"
	"fmt"
)

type Controller interface {
	Resource() string
	Reconcile(context.Context) error
}

type Daemon struct {
	Controllers []Controller
}

func (d Daemon) Reconcile(ctx context.Context) error {
	if len(d.Controllers) == 0 {
		return nil
	}
	var err error
	for _, controller := range d.Controllers {
		if ctxErr := ctx.Err(); ctxErr != nil {
			return ctxErr
		}
		if controller == nil {
			err = errors.Join(err, errors.New("daemon controller is nil"))
			continue
		}
		if reconcileErr := controller.Reconcile(ctx); reconcileErr != nil {
			err = errors.Join(err, fmt.Errorf("reconcile %s: %w", controller.Resource(), reconcileErr))
		}
	}
	return err
}
