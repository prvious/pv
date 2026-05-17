package cli

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/host"
	"github.com/prvious/pv/internal/project"
	"github.com/prvious/pv/internal/status"
)

func collectStatusEntries(ctx context.Context, paths host.Paths, store *control.FileStore) ([]status.Entry, error) {
	entries, err := linkedProjectStatusEntries(ctx, paths)
	if err != nil {
		return nil, err
	}
	installEntries, err := installStatusEntries(ctx, paths, store)
	if err != nil {
		return nil, err
	}
	return append(entries, installEntries...), nil
}

func linkedProjectStatusEntries(ctx context.Context, paths host.Paths) ([]status.Entry, error) {
	state, ok, err := project.Registry{Path: projectStatePath(paths)}.Current(ctx)
	if err != nil || !ok {
		return nil, err
	}

	name := projectStatusName(state)
	entries := []status.Entry{
		{
			View:       status.ViewProject,
			Name:       name,
			State:      status.StateUnknown,
			Desired:    fmt.Sprintf("path=%s php=%s hosts=%s services=%s", state.Path, state.PHP, joinStatusValues(state.Hosts), joinStatusValues(state.Services)),
			Observed:   "pending reconciliation",
			LogPath:    filepath.Join(paths.Root(), "logs", "project", name+".log"),
			NextAction: "run reconciliation",
		},
		{
			View:       status.ViewRuntime,
			Name:       control.ResourcePHP,
			State:      status.StateMissingInstall,
			Desired:    fmt.Sprintf("php %s", state.PHP),
			Observed:   "pending reconciliation",
			LogPath:    versionedLogPath(paths, control.ResourcePHP, state.PHP),
			NextAction: fmt.Sprintf("run pv php:install %s", state.PHP),
		},
	}
	for _, service := range state.Services {
		entries = append(entries, status.Entry{
			View:       status.ViewResource,
			Name:       service,
			State:      status.StateMissingInstall,
			Desired:    service + " declared",
			Observed:   "pending reconciliation",
			LogPath:    filepath.Join(paths.Root(), "logs", service, "declared.log"),
			LastError:  service + " is not installed",
			NextAction: missingResourceInstallAction(service),
			Values:     resourceStatusValues(service),
		})
	}
	for _, projectHost := range state.Hosts {
		entries = append(entries, status.Entry{
			View:       status.ViewGateway,
			Name:       projectHost,
			State:      status.StateUnknown,
			Desired:    "https://" + projectHost,
			Observed:   "pending reconciliation",
			LogPath:    filepath.Join(paths.Root(), "logs", "gateway", projectHost+".log"),
			NextAction: "run reconciliation",
		})
	}
	return entries, nil
}

func installStatusEntries(ctx context.Context, paths host.Paths, store *control.FileStore) ([]status.Entry, error) {
	var entries []status.Entry
	for _, resource := range []string{
		control.ResourcePHP,
		control.ResourceComposer,
		control.ResourceMago,
		control.ResourcePostgres,
		control.ResourceMySQL,
		control.ResourceRedis,
		control.ResourceMailpit,
		control.ResourceRustFS,
	} {
		desired, desiredOK, err := store.Desired(ctx, resource)
		if err != nil {
			return nil, err
		}
		if !desiredOK {
			continue
		}
		entry := status.Entry{
			View:       statusViewForResource(resource),
			Name:       resource,
			State:      status.StateUnknown,
			Desired:    desiredResourceText(desired),
			Observed:   resource + " pending",
			LogPath:    versionedLogPath(paths, resource, desired.Version),
			NextAction: "run reconciliation",
		}
		observed, observedOK, err := store.Observed(ctx, resource)
		if err != nil {
			return nil, err
		}
		if observedOK {
			entry.State = normalizedStatusState(observed.State)
			entry.Observed = observedStatusText(observed)
			entry.LastReconcile = observed.LastReconcileTime
			entry.LastError = observed.LastError
			entry.NextAction = observed.NextAction
		} else if desired.Resource == control.ResourceComposer && desired.RuntimeVersion != "" {
			phpObserved, phpOK, err := store.Observed(ctx, control.ResourcePHP)
			if err != nil {
				return nil, err
			}
			if !phpOK || phpObserved.DesiredVersion != desired.RuntimeVersion || phpObserved.State != control.StateReady {
				entry.State = status.StateBlocked
				entry.Observed = fmt.Sprintf("%s %s blocked", desired.Resource, desired.Version)
				entry.LastError = fmt.Sprintf("PHP runtime %s is not installed", desired.RuntimeVersion)
				entry.NextAction = fmt.Sprintf("run pv php:install %s", desired.RuntimeVersion)
			}
		}
		entries = append(entries, entry)
	}
	return entries, nil
}

func projectStatePath(paths host.Paths) string {
	return filepath.Join(paths.Root(), "state", "project.json")
}

func projectStatusName(state project.State) string {
	if len(state.Hosts) > 0 && strings.TrimSpace(state.Hosts[0]) != "" {
		return state.Hosts[0]
	}
	return filepath.Base(state.Path)
}

func joinStatusValues(values []string) string {
	if len(values) == 0 {
		return "none"
	}
	return strings.Join(values, ",")
}

func resourceStatusValues(resource string) map[string]string {
	switch resource {
	case control.ResourceMailpit:
		return map[string]string{
			"MAIL_HOST":   "127.0.0.1",
			"MAIL_MAILER": "smtp",
			"MAIL_PORT":   "1025",
		}
	case control.ResourcePostgres:
		return map[string]string{
			"DB_CONNECTION": "pgsql",
			"DB_HOST":       "127.0.0.1",
			"DB_PORT":       "5432",
		}
	case control.ResourceMySQL:
		return map[string]string{
			"DB_CONNECTION": "mysql",
			"DB_HOST":       "127.0.0.1",
			"DB_PORT":       "3306",
		}
	case control.ResourceRedis:
		return map[string]string{
			"REDIS_HOST": "127.0.0.1",
			"REDIS_PORT": "6379",
		}
	case control.ResourceRustFS:
		return map[string]string{
			"AWS_ENDPOINT_URL":      "http://127.0.0.1:9000",
			"AWS_SECRET_ACCESS_KEY": "local-rustfs-secret",
		}
	default:
		return nil
	}
}

func missingResourceInstallAction(resource string) string {
	switch resource {
	case control.ResourcePostgres:
		return "run pv postgres:install <version>"
	case control.ResourceMySQL:
		return "run pv mysql:install <version>"
	case control.ResourceRedis:
		return "run pv redis:install <version>"
	case control.ResourceMailpit:
		return "run pv mailpit:install <version>"
	case control.ResourceRustFS:
		return "run pv rustfs:install <version>"
	default:
		return "install the declared resource and run reconciliation"
	}
}

func statusViewForResource(resource string) status.View {
	switch resource {
	case control.ResourcePHP, control.ResourceComposer, control.ResourceMago:
		return status.ViewRuntime
	default:
		return status.ViewResource
	}
}

func normalizedStatusState(state string) status.State {
	switch state {
	case control.StateReady:
		return status.StateHealthy
	case control.StateBlocked:
		return status.StateBlocked
	case control.StateMissing:
		return status.StateMissingInstall
	case control.StateStopped:
		return status.StateStopped
	case control.StateFailed:
		return status.StateFailed
	default:
		return status.StateUnknown
	}
}

func versionedLogPath(paths host.Paths, resource string, version string) string {
	logPath, err := paths.LogPath(resource, version)
	if err != nil {
		return ""
	}
	return logPath
}

func desiredResourceText(desired control.DesiredResource) string {
	if desired.RuntimeVersion != "" {
		return fmt.Sprintf("%s %s install with php %s", desired.Resource, desired.Version, desired.RuntimeVersion)
	}
	return fmt.Sprintf("%s %s install", desired.Resource, desired.Version)
}

func observedStatusText(observed control.ObservedStatus) string {
	return fmt.Sprintf("%s %s %s", observed.Resource, observed.DesiredVersion, observed.State)
}
