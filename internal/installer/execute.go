package installer

import (
	"context"
	"errors"
)

type Installer interface {
	Install(context.Context, Item) error
}

type ResultState string

const (
	ResultReady   ResultState = "ready"
	ResultSkipped ResultState = "skipped"
	ResultFailed  ResultState = "failed"
)

type InstallResult struct {
	ID     Identity
	State  ResultState
	Reason string
	Err    error
}

func Execute(ctx context.Context, plan Plan, installer Installer) ([]InstallResult, error) {
	if installer == nil {
		return nil, errors.New("installer is required")
	}
	if err := plan.Validate(); err != nil {
		return nil, err
	}
	ordered, err := plan.Order()
	if err != nil {
		return nil, err
	}

	results := make([]InstallResult, 0, len(ordered))
	failed := make(map[string]error)
	for _, item := range ordered {
		if err := ctx.Err(); err != nil {
			results = append(results, InstallResult{ID: item.ID, State: ResultFailed, Reason: "context cancelled", Err: err})
			failed[item.ID.String()] = err
			continue
		}
		if dependencyErr := firstFailedDependency(item, failed); dependencyErr != nil {
			results = append(results, InstallResult{
				ID:     item.ID,
				State:  ResultSkipped,
				Reason: "dependency failed",
				Err:    dependencyErr,
			})
			failed[item.ID.String()] = dependencyErr
			continue
		}
		if err := installer.Install(ctx, item); err != nil {
			results = append(results, InstallResult{ID: item.ID, State: ResultFailed, Reason: err.Error(), Err: err})
			failed[item.ID.String()] = err
			continue
		}
		results = append(results, InstallResult{ID: item.ID, State: ResultReady})
	}
	return results, nil
}

func firstFailedDependency(item Item, failed map[string]error) error {
	for _, dependency := range item.DependsOn {
		if err, ok := failed[dependency.String()]; ok {
			return err
		}
	}
	return nil
}
