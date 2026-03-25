package service

import (
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/container"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/spf13/cobra"
)

var logsCmd = &cobra.Command{
	Use:     "service:logs <service>",
	GroupID: "service",
	Short:   "Tail container logs for a service",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		key := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}
		key, resolveErr := reg.ResolveServiceKey(key)
		if resolveErr != nil {
			return resolveErr
		}

		if reg.FindService(key) == nil {
			return fmt.Errorf("service %q not found", key)
		}

		svcName := extractServiceName(key)
		version := extractVersion(key)
		svc, err := services.Lookup(svcName)
		if err != nil {
			return err
		}
		containerName := svc.ContainerName(version)

		engine, err := container.NewEngine(config.ColimaSocketPath())
		if err != nil {
			return fmt.Errorf("cannot connect to Docker: %w", err)
		}
		defer engine.Close()

		running, err := engine.IsRunning(cmd.Context(), containerName)
		if err != nil {
			return fmt.Errorf("cannot check if %s is running: %w", key, err)
		}
		if !running {
			return fmt.Errorf("service %q is not running, start it first: pv service:start %s", key, key)
		}

		return engine.Logs(cmd.Context(), containerName, os.Stderr)
	},
}
