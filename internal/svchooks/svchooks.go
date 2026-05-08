// Package svchooks contains the project-binding and env-fallback hooks
// shared by binary-service commands (rustfs:*, mailpit:*) and by the
// remaining docker-service paths (service:stop / service:remove /
// service:destroy). It exists to break a would-be import cycle: the
// rustfs and mailpit command packages must not depend on
// internal/commands/service since the goal of that move is precisely
// to take them out of that namespace.
package svchooks

import (
	"fmt"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/ui"
)

// UpdateLinkedProjectsEnvBinary updates .env for Laravel projects linked
// to a binary service (s3, mail) when the service is added or restarted.
// Mirrors updateLinkedProjectsEnv in internal/commands/service/hooks.go
// for docker services; the only difference is which laravel helper it
// calls (the binary variant doesn't need a port argument because
// BinaryService.Port() is fixed at the struct level).
func UpdateLinkedProjectsEnvBinary(reg *registry.Registry, svcName string, svc services.BinaryService) {
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
		if err := laravel.UpdateProjectEnvForBinaryService(
			p.Path, p.Name, svcName, svc, project.Services,
		); err != nil {
			ui.Subtle(fmt.Sprintf("Could not update .env for %s: %v", p.Name, err))
		} else {
			ui.Success(fmt.Sprintf("Updated .env for %s", p.Name))
		}
	}
}

// BindBinaryServiceToAllProjects sets the per-project Services flag for
// svcName on every Laravel project so UpdateLinkedProjectsEnvBinary can
// find projects that were linked before the service existed. Returns an
// error for unknown svcName so new binary services can't silently skip
// this step — the set of cases here must stay in lockstep with
// registry.ProjectServices fields.
func BindBinaryServiceToAllProjects(reg *registry.Registry, svcName string) error {
	switch svcName {
	case "mail", "s3":
	default:
		return fmt.Errorf("BindBinaryServiceToAllProjects: unknown binary service %q (add a case here and a field on ProjectServices)", svcName)
	}
	for i := range reg.Projects {
		p := &reg.Projects[i]
		if p.Type != "laravel" && p.Type != "laravel-octane" {
			continue
		}
		if p.Services == nil {
			p.Services = &registry.ProjectServices{}
		}
		switch svcName {
		case "mail":
			p.Services.Mail = true
		case "s3":
			p.Services.S3 = true
		}
	}
	return nil
}

// ApplyFallbacksToLinkedProjects applies safe env fallbacks when a
// service is stopped or removed. Used by both binary-service commands
// (rustfs:uninstall, mailpit:uninstall) and the docker-service paths
// (service:stop / service:remove / service:destroy).
func ApplyFallbacksToLinkedProjects(reg *registry.Registry, svcName string) {
	settings, err := config.LoadSettings()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load settings for service fallback hooks: %v", err))
		return
	}
	if settings.Automation.ServiceFallback == config.AutoOff {
		return
	}

	projectNames := reg.ProjectsUsingService(svcName)
	if len(projectNames) == 0 {
		return
	}

	shouldFallback := settings.Automation.ServiceFallback == config.AutoOn
	if settings.Automation.ServiceFallback == config.AutoAsk {
		if !automation.IsInteractive() {
			return
		}
		confirmed, err := automation.ConfirmFunc(
			fmt.Sprintf("Apply env fallbacks for %s to %d project(s)", svcName, len(projectNames)),
		)
		if err != nil {
			return
		}
		shouldFallback = confirmed
	}
	if !shouldFallback {
		return
	}

	for _, pName := range projectNames {
		project := reg.Find(pName)
		if project == nil {
			continue
		}
		envPath := filepath.Join(project.Path, ".env")
		if err := laravel.ApplyFallbacks(envPath, svcName); err != nil {
			ui.Subtle(fmt.Sprintf("Could not apply fallbacks for %s: %v", pName, err))
		} else {
			ui.Success(fmt.Sprintf("Applied %s fallbacks for %s", svcName, pName))
		}
	}
}
