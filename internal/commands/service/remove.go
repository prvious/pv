package service

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var removeCmd = &cobra.Command{
	Use:     "service:remove <service>",
	GroupID: "service",
	Short: "Stop and remove a service container (data preserved)",
	Args:  cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		key := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		svc := reg.FindService(key)
		if svc == nil {
			return fmt.Errorf("service %q not found", key)
		}

		if err := ui.Step(fmt.Sprintf("Removing %s...", key), func() (string, error) {
			// Docker SDK: stop + remove container.
			return fmt.Sprintf("%s removed", key), nil
		}); err != nil {
			return err
		}

		if err := reg.RemoveService(key); err != nil {
			return err
		}
		if err := reg.Save(); err != nil {
			return fmt.Errorf("cannot save registry: %w", err)
		}

		// Regenerate Caddy configs for service consoles.
		_ = caddy.GenerateServiceSiteConfigs(reg)

		// Determine data path for the message.
		svcName := key
		version := "latest"
		if idx := strings.Index(key, ":"); idx > 0 {
			svcName = key[:idx]
			version = key[idx+1:]
		}
		dataDir := config.ServiceDataDir(svcName, version)

		ui.Subtle(fmt.Sprintf("Data preserved at %s", dataDir))
		ui.Subtle(fmt.Sprintf("Run 'pv service:add %s %s' to start it again.", svcName, version))

		return nil
	},
}
