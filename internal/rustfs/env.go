package rustfs

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/laravel"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/ui"
)

// UpdateLinkedProjectsEnv updates .env for Laravel projects linked to
// rustfs (s3) when the service is added or restarted. Gates by the
// settings.Automation.ServiceEnvUpdate flag (off / on / ask).
func UpdateLinkedProjectsEnv(reg *registry.Registry) {
	settings, err := config.LoadSettings()
	if err != nil {
		ui.Subtle(fmt.Sprintf("Could not load settings for service env hooks: %v", err))
		return
	}
	if settings.Automation.ServiceEnvUpdate == config.AutoOff {
		return
	}

	linkedNames := reg.ProjectsUsingService(ServiceKey())
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
		if err := UpdateProjectEnv(p.Path, p.Name, project.Services); err != nil {
			ui.Subtle(fmt.Sprintf("Could not update .env for %s: %v", p.Name, err))
		} else {
			ui.Success(fmt.Sprintf("Updated .env for %s", p.Name))
		}
	}
}

// UpdateProjectEnv merges rustfs connection vars + smart Laravel vars
// into the project's .env. Skips silently if .env doesn't exist.
func UpdateProjectEnv(projectPath, projectName string, bound *registry.ProjectServices) error {
	envPath := filepath.Join(projectPath, ".env")
	if _, err := os.Stat(envPath); os.IsNotExist(err) {
		return nil
	}
	allVars := EnvVars(projectName)
	smartVars := laravel.SmartEnvVars(bound)
	for k, v := range smartVars {
		allVars[k] = v
	}
	backupPath := envPath + ".pv-backup"
	return projectenv.MergeDotEnv(envPath, backupPath, allVars)
}
