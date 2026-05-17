package installer

import "context"

func PersistThenSignal(ctx context.Context, persist func(context.Context) error, signal func(context.Context) error) error {
	if err := persist(ctx); err != nil {
		return err
	}
	if signal == nil {
		return nil
	}
	return signal(ctx)
}
