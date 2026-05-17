package rustfs

import (
	"fmt"
	"net/http"
	"time"

	"github.com/prvious/pv/internal/caddy"
	pkg "github.com/prvious/pv/internal/rustfs"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var installCmd = &cobra.Command{
	Use:     "rustfs:install [version]",
	GroupID: "rustfs",
	Short:   "Install RustFS (S3-compatible storage) and start it",
	Long:    "Downloads the versioned RustFS artifact, sets it as wanted-running, and signals the daemon to start it.",
	Example: `pv rustfs:install
pv s3:install`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		version := ""
		if len(args) > 0 {
			version = args[0]
		}
		resolved, err := pkg.ResolveVersion(version)
		if err != nil {
			return err
		}
		client := &http.Client{Timeout: 5 * time.Minute}
		if pkg.IsInstalled(resolved) {
			if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
				return err
			}
			ui.Success(fmt.Sprintf("%s %s already installed - marked as wanted running.", pkg.DisplayName(), resolved))
			return signalDaemon(pkg.DisplayName())
		}
		if err := ui.StepProgress(fmt.Sprintf("Installing %s %s...", pkg.DisplayName(), resolved), func(progress func(written, total int64)) (string, error) {
			if err := pkg.InstallProgress(client, resolved, progress); err != nil {
				return "", err
			}
			return fmt.Sprintf("Installed %s %s", pkg.DisplayName(), resolved), nil
		}); err != nil {
			return err
		}
		if err := caddy.GenerateServiceSiteConfigs(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
		}
		if err := signalDaemon(pkg.DisplayName()); err != nil {
			return err
		}
		printConnectionDetails(resolved)
		return nil
	},
}
