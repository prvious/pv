package service

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"

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

		kind, binSvc, _, kindErr := resolveKind(reg, args[0])
		if kindErr != nil {
			return kindErr
		}
		if kind == kindBinary {
			logPath := filepath.Join(config.PvDir(), "logs", binSvc.Binary().Name+".log")
			f, err := os.Open(logPath)
			if err != nil {
				if os.IsNotExist(err) {
					return fmt.Errorf("no log file yet (%s). Has the service run?", logPath)
				}
				return err
			}
			defer f.Close()
			// Dump existing content.
			if _, err := io.Copy(os.Stdout, f); err != nil {
				return err
			}
			// Follow mode (like tail -f). Poll every 250ms for new data; exit on Ctrl-C.
			for {
				select {
				case <-cmd.Context().Done():
					return nil
				case <-time.After(250 * time.Millisecond):
				}
				if _, err := io.Copy(os.Stdout, f); err != nil {
					if err == io.EOF {
						continue
					}
					return err
				}
			}
		}

		key, resolveErr := reg.ResolveServiceKey(key)
		if resolveErr != nil {
			return resolveErr
		}

		if svc, findErr := reg.FindService(key); findErr != nil {
			return findErr
		} else if svc == nil {
			return fmt.Errorf("service %q not found", key)
		}

		svcName, version := services.ParseServiceKey(key)
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
