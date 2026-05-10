package postgres

import (
	"fmt"

	"github.com/prvious/pv/internal/laravel"
	pg "github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

const defaultMajor = "18"

var installCmd = &cobra.Command{
	Use:     "postgres:install [major]",
	GroupID: "postgres",
	Short:   "Install (or re-install) a PostgreSQL major",
	Long:    "Downloads PostgreSQL binaries, runs initdb, and registers the major as wanted-running. Default major: 18.",
	Example: `# Install PostgreSQL 18 (default)
pv postgres:install

# Install PostgreSQL 17 alongside 18
pv postgres:install 17`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		major := defaultMajor
		if len(args) > 0 {
			major = args[0]
		}

		// If already on disk, refresh runtime state (conf overrides, hba,
		// socket dir) idempotently and mark wanted=running. This guards
		// against an /tmp socket dir reaped between boots and keeps
		// pv-managed conf in sync if the defaults changed across releases.
		if pg.IsInstalled(major) {
			if err := pg.EnsureRuntime(major); err != nil {
				return err
			}
			if err := pg.SetWanted(major, pg.WantedRunning); err != nil {
				return err
			}
			if err := bindLinkedProjectsToPostgres(major); err != nil {
				ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
			}
			ui.Success(fmt.Sprintf("PostgreSQL %s already installed — marked as wanted running.", major))
			return signalDaemon()
		}

		// Run the download/install pipeline.
		if err := downloadCmd.RunE(downloadCmd, []string{major}); err != nil {
			return err
		}
		if err := bindLinkedProjectsToPostgres(major); err != nil {
			ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s installed.", major))
		return signalDaemon()
	},
}

// bindLinkedProjectsToPostgres walks linked projects and binds any
// pgsql-using project to the just-installed major if it has no postgres
// binding yet. The retroactive-bind path for projects linked before
// postgres existed; mirrors rustfs.BindToAllProjects / mailpit.BindToAllProjects.
//
// Bind condition: project is Laravel-shaped AND its .env has
// DB_CONNECTION=pgsql AND Services.Postgres is empty. Projects already
// bound to a different major are left alone.
func bindLinkedProjectsToPostgres(major string) error {
	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("load registry: %w", err)
	}
	changed := false
	for i := range reg.Projects {
		p := &reg.Projects[i]
		if p.Type != "laravel" && p.Type != "laravel-octane" {
			continue
		}
		if p.Services != nil && p.Services.Postgres != "" {
			continue // already bound (to this or another major)
		}
		envPath := p.Path + "/.env"
		envVars, err := projectenv.ReadDotEnv(envPath)
		if err != nil {
			continue // no .env or unreadable; skip silently
		}
		if envVars["DB_CONNECTION"] != "pgsql" {
			continue
		}
		if p.Services == nil {
			p.Services = &registry.ProjectServices{}
		}
		p.Services.Postgres = major
		changed = true
		if err := laravel.UpdateProjectEnvForPostgres(p.Path, p.Name, major, p.Services); err != nil {
			ui.Subtle(fmt.Sprintf("Could not write postgres env vars for %s: %v", p.Name, err))
		}
	}
	if changed {
		if err := reg.Save(); err != nil {
			return fmt.Errorf("save registry: %w", err)
		}
	}
	return nil
}

func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — postgres will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
