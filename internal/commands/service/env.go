package service

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var envCmd = &cobra.Command{
	Use:     "service:env [service]",
	GroupID: "service",
	Short:   "Print environment variables for a service",
	Args:    cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		// Determine project name from current directory.
		cwd, cwdErr := os.Getwd()
		if cwdErr != nil {
			return fmt.Errorf("cannot determine current directory: %w", cwdErr)
		}
		projectName := services.SanitizeProjectName(filepath.Base(cwd))

		if len(args) == 0 {
			// Print env for all services.
			svcs := reg.ListServices()
			if len(svcs) == 0 {
				fmt.Fprintln(os.Stderr)
				ui.Subtle("No services configured.")
				fmt.Fprintln(os.Stderr)
				return nil
			}

			fmt.Fprintln(os.Stderr)
			for key, instance := range svcs {
				svcName, _ := services.ParseServiceKey(key)
				envVars, err := envVarsFor(svcName, projectName, instance.Port)
				if err != nil {
					ui.Subtle(fmt.Sprintf("Skipping unknown service %q", svcName))
					continue
				}
				printEnvVars(key, envVars)
			}
			return nil
		}

		key := args[0]
		var resolveErr error
		key, resolveErr = reg.ResolveServiceKey(key)
		if resolveErr != nil {
			return resolveErr
		}
		instance, findErr := reg.FindService(key)
		if findErr != nil {
			return findErr
		}
		if instance == nil {
			return fmt.Errorf("service %q not found", key)
		}

		svcName, _ := services.ParseServiceKey(key)
		envVars, err := envVarsFor(svcName, projectName, instance.Port)
		if err != nil {
			return err
		}

		fmt.Fprintln(os.Stderr)
		printEnvVars(key, envVars)

		return nil
	},
}

// envVarsFor resolves a service name across both registries and returns the
// .env keys/values it injects into a linked project. Used by both the
// all-services and single-service code paths in envCmd so the per-kind
// dispatch lives in one place.
//
// Binary services have EnvVars(projectName) — port is fixed by the binary
// itself. Docker services have EnvVars(projectName, port) — port comes from
// the registry.ServiceInstance.
func envVarsFor(svcName, projectName string, port int) (map[string]string, error) {
	kind, binSvc, docSvc, err := services.LookupAny(svcName)
	if err != nil {
		return nil, err
	}
	switch kind {
	case services.KindBinary:
		return binSvc.EnvVars(projectName), nil
	case services.KindDocker:
		return docSvc.EnvVars(projectName, port), nil
	}
	return nil, fmt.Errorf("unexpected kind %v for %q", kind, svcName)
}

func printEnvVars(key string, envVars map[string]string) {
	fmt.Fprintf(os.Stderr, "  %s\n", ui.Muted.Render("# "+key))
	for k, v := range envVars {
		fmt.Fprintf(os.Stderr, "  %s=%s\n", k, v)
	}
	fmt.Fprintln(os.Stderr)
}
