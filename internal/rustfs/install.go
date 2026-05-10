package rustfs

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
	"github.com/prvious/pv/internal/ui"
)

// Install downloads the rustfs binary, registers it, retroactively binds
// it to existing Laravel projects, writes their .env vars, and signals
// the daemon to reconcile. Idempotent on already-registered.
func Install() error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("cannot load registry: %w", err)
	}
	if _, exists := reg.Services[ServiceKey()]; exists {
		ui.Success(fmt.Sprintf("%s is already added", DisplayName()))
		return nil
	}

	client := &http.Client{Timeout: 60 * time.Second}

	latest, err := binaries.FetchLatestVersion(client, Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
	}
	if err := ui.Step(fmt.Sprintf("Downloading %s %s...", Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	enabled := true
	inst := &registry.ServiceInstance{
		Port:        Port(),
		ConsolePort: ConsolePort(),
		Enabled:     &enabled,
	}
	if err := reg.AddService(ServiceKey(), inst); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	BindToAllProjects(reg)
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry after binding: %w", err)
	}

	UpdateLinkedProjectsEnv(reg)

	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
	}

	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
		ui.Success(fmt.Sprintf("%s registered and running on :%d", DisplayName(), Port()))
	} else {
		ui.Success(fmt.Sprintf("%s registered on :%d", DisplayName(), Port()))
		ui.Subtle("daemon not running — service will start on next `pv start`")
	}

	printConnectionDetails()
	return nil
}

func printConnectionDetails() {
	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "    %s  127.0.0.1\n", ui.Muted.Render("Host"))
	fmt.Fprintf(os.Stderr, "    %s  %d\n", ui.Muted.Render("Port"), Port())
	settings, _ := config.LoadSettings()
	if settings != nil {
		for _, route := range WebRoutes() {
			fmt.Fprintf(os.Stderr, "    %s  https://%s.pv.%s\n",
				ui.Muted.Render(route.Subdomain), route.Subdomain, settings.Defaults.TLD)
		}
	}
	fmt.Fprintln(os.Stderr)
}
