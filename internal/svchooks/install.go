package svchooks

import (
	"fmt"
	"net/http"
	"os"
	"time"

	"github.com/prvious/pv/internal/binaries"
	"github.com/prvious/pv/internal/caddy"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
)

// Install downloads svc's binary (if not yet present), records its
// version, registers the service, retroactively binds it to existing
// Laravel projects, writes their .env vars, and signals the daemon to
// reconcile. Idempotent on the registered case (returns nil after a
// "%s is already added" notice).
func Install(reg *registry.Registry, svc services.BinaryService) error {
	name := svc.Name()
	if existing, exists := reg.Services[name]; exists {
		if existing.Kind != "binary" {
			return fmt.Errorf(
				"%s is registered as %q from a previous pv version. "+
					"Run `pv uninstall && pv setup` to reset",
				name, existing.Kind,
			)
		}
		ui.Success(fmt.Sprintf("%s is already added", svc.DisplayName()))
		return nil
	}

	client := &http.Client{Timeout: 60 * time.Second}

	latest, err := binaries.FetchLatestVersion(client, svc.Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", svc.Binary().DisplayName, err)
	}

	if err := ui.Step(fmt.Sprintf("Downloading %s %s...", svc.Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, svc.Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", svc.Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(svc.Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	enabled := true
	inst := &registry.ServiceInstance{
		Port:        svc.Port(),
		ConsolePort: svc.ConsolePort(),
		Kind:        "binary",
		Enabled:     &enabled,
	}
	if err := reg.AddService(name, inst); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	// Projects linked before this service existed need their per-project
	// flag set so subsequent .env writes find them via ProjectsUsingService.
	if err := BindBinaryServiceToAllProjects(reg, name); err != nil {
		return fmt.Errorf("cannot bind service to projects: %w", err)
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry after binding service: %w", err)
	}

	UpdateLinkedProjectsEnvBinary(reg, name, svc)

	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
	}

	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
		ui.Success(fmt.Sprintf("%s registered and running on :%d", svc.DisplayName(), svc.Port()))
	} else {
		ui.Success(fmt.Sprintf("%s registered on :%d", svc.DisplayName(), svc.Port()))
		ui.Subtle("daemon not running — service will start on next `pv start`")
	}

	PrintConnectionDetails(svc)
	return nil
}

// PrintConnectionDetails writes the verbose "Host / Port / web routes"
// footer mirroring the docker addService output, scoped to the
// binary-service shape.
func PrintConnectionDetails(svc services.BinaryService) {
	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "    %s  127.0.0.1\n", ui.Muted.Render("Host"))
	fmt.Fprintf(os.Stderr, "    %s  %d\n", ui.Muted.Render("Port"), svc.Port())
	settings, _ := config.LoadSettings()
	if settings != nil {
		for _, route := range svc.WebRoutes() {
			fmt.Fprintf(os.Stderr, "    %s  https://%s.pv.%s\n",
				ui.Muted.Render(route.Subdomain), route.Subdomain, settings.Defaults.TLD)
		}
	}
	fmt.Fprintln(os.Stderr)
}
