package rustfs

import (
	"fmt"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
)

// ApplyFallbacksToLinkedProjects applies safe env fallbacks to linked
// Laravel projects when rustfs is being uninstalled or stopped. Gates
// by the settings.Automation.ServiceFallback flag (off / on / ask).
func ApplyFallbacksToLinkedProjects(reg *registry.Registry) {
	settings, err := config.LoadSettings()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load settings for service fallback hooks: %v", err))
		return
	}
	if settings.Automation.ServiceFallback == config.AutoOff {
		return
	}

	projectNames := reg.ProjectsUsingService(serviceKey)
	if len(projectNames) == 0 {
		return
	}

	shouldFallback := settings.Automation.ServiceFallback == config.AutoOn
	if settings.Automation.ServiceFallback == config.AutoAsk {
		if !automation.IsInteractive() {
			return
		}
		confirmed, err := automation.ConfirmFunc(
			fmt.Sprintf("Apply env fallbacks for %s to %d project(s)", serviceKey, len(projectNames)),
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
		if err := laravel.ApplyFallbacks(envPath, serviceKey); err != nil {
			ui.Subtle(fmt.Sprintf("Could not apply fallbacks for %s: %v", pName, err))
		} else {
			ui.Success(fmt.Sprintf("Applied %s fallbacks for %s", serviceKey, pName))
		}
	}
}
