package service

import (
	"fmt"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/svchooks"
	"github.com/prvious/pv/internal/ui"
)

// updateLinkedProjectsEnv updates .env for Laravel projects linked to the
// given service (including Octane) when a service is added or started.
func updateLinkedProjectsEnv(reg *registry.Registry, svcName string, svc services.Service, version string) {
	settings, err := config.LoadSettings()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load settings for service env hooks: %v", err))
		return
	}
	if settings.Automation.ServiceEnvUpdate == config.AutoOff {
		return
	}

	linkedNames := reg.ProjectsUsingService(svcName)
	var laravelProjects []registry.Project
	for _, name := range linkedNames {
		p := reg.Find(name)
		if p != nil && (p.Type == "laravel" || p.Type == "laravel-octane") {
			laravelProjects = append(laravelProjects, *p)
		}
	}
	if len(laravelProjects) == 0 {
		return
	}

	shouldUpdate := settings.Automation.ServiceEnvUpdate == config.AutoOn
	if settings.Automation.ServiceEnvUpdate == config.AutoAsk {
		if !automation.IsInteractive() {
			return
		}
		confirmed, err := automation.ConfirmFunc(
			fmt.Sprintf("Update .env for %d linked Laravel project(s)", len(laravelProjects)),
		)
		if err != nil {
			return
		}
		shouldUpdate = confirmed
	}
	if !shouldUpdate {
		return
	}

	for _, p := range laravelProjects {
		project := reg.Find(p.Name)
		if project == nil || project.Services == nil {
			continue
		}
		if err := laravel.UpdateProjectEnvForService(
			p.Path, p.Name, svcName, svc, svc.Port(version), project.Services,
		); err != nil {
			ui.Subtle(fmt.Sprintf("Could not update .env for %s: %v", p.Name, err))
		} else {
			ui.Success(fmt.Sprintf("Updated .env for %s", p.Name))
		}
	}
}

// applyStopAllFallbacks applies env fallbacks for every Docker service in the
// registry. Binary services are skipped because the stop-all command does not
// stop them — they're owned by the rustfs:* / mailpit:* commands now.
// Called from the no-args service:stop path after stopping all Docker
// containers.
func applyStopAllFallbacks(reg *registry.Registry) {
	for key, inst := range reg.ListServices() {
		if inst.Kind == "binary" {
			continue
		}
		svcName, _ := services.ParseServiceKey(key)
		svchooks.ApplyFallbacksToLinkedProjects(reg, svcName)
	}
}
