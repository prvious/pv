package mysql

import (
	"fmt"

	"github.com/prvious/pv/internal/laravel"
	my "github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

const defaultVersion = "8.4"

var installCmd = &cobra.Command{
	Use:     "mysql:install [version]",
	GroupID: "mysql",
	Short:   "Install (or re-install) a MySQL version",
	Long:    "Downloads MySQL binaries, runs --initialize-insecure on first install, and registers the version as wanted-running. Default version: 8.4.",
	Example: `# Install MySQL 8.4 (default)
pv mysql:install

# Install MySQL 9.7 alongside 8.4
pv mysql:install 9.7`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := defaultVersion
		if len(args) > 0 {
			version = args[0]
		}

		// Already installed → idempotent: re-mark wanted=running and
		// signal the daemon. Same friendly contract postgres uses.
		if my.IsInstalled(version) {
			if err := my.SetWanted(version, my.WantedRunning); err != nil {
				return err
			}
			if err := bindLinkedProjectsToMysql(version); err != nil {
				ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
			}
			ui.Success(fmt.Sprintf("MySQL %s already installed — marked as wanted running.", version))
			return signalDaemon()
		}

		// Run the download/extract/initdb pipeline.
		if err := downloadCmd.RunE(downloadCmd, []string{version}); err != nil {
			return err
		}
		if err := bindLinkedProjectsToMysql(version); err != nil {
			ui.Subtle(fmt.Sprintf("Could not retroactively bind linked projects: %v", err))
		}
		ui.Success(fmt.Sprintf("MySQL %s installed.", version))
		return signalDaemon()
	},
}

// bindLinkedProjectsToMysql walks linked projects and binds any
// mysql-using project to the just-installed version if it has no mysql
// binding yet. Mirrors postgres' bindLinkedProjectsToPostgres — the
// retroactive-bind path for projects linked before mysql existed.
//
// Bind condition: project is Laravel-shaped AND its .env has
// DB_CONNECTION=mysql AND Services.MySQL is empty. Projects already
// bound to a different version are left alone. An unset DB_CONNECTION
// does NOT trigger binding — "mysql" is Laravel's compiled default and
// we don't step on undecided projects.
func bindLinkedProjectsToMysql(version string) error {
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
		if p.Services != nil && p.Services.MySQL != "" {
			continue
		}
		envPath := p.Path + "/.env"
		envVars, err := projectenv.ReadDotEnv(envPath)
		if err != nil {
			continue
		}
		if envVars["DB_CONNECTION"] != "mysql" {
			continue
		}
		if p.Services == nil {
			p.Services = &registry.ProjectServices{}
		}
		p.Services.MySQL = version
		changed = true
		if err := laravel.UpdateProjectEnvForMysql(p.Path, p.Name, version, p.Services); err != nil {
			ui.Subtle(fmt.Sprintf("Could not write mysql env vars for %s: %v", p.Name, err))
		}
	}
	if changed {
		if err := reg.Save(); err != nil {
			return fmt.Errorf("save registry: %w", err)
		}
	}
	return nil
}

// signalDaemon nudges the running pv daemon to reconcile, or no-ops with
// a friendly note if the daemon isn't up.
func signalDaemon() error {
	if !server.IsRunning() {
		ui.Subtle("daemon not running — mysql will start on next `pv start`")
		return nil
	}
	return server.SignalDaemon()
}
