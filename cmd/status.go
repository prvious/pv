package cmd

import (
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/daemon"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:     "status",
	GroupID: "core",
	Short:   "Show pv server status",
	RunE: func(cmd *cobra.Command, args []string) error {
		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}

		ui.Header(version)

		// Determine running state and mode.
		var running bool
		var mode string
		var pid int

		if daemon.IsLoaded() {
			running = true
			mode = "daemon"
			pid, _ = daemon.GetPID()
		} else if server.IsRunning() {
			running = true
			mode = "foreground"
			pid, _ = server.ReadPID()
		}

		if running {
			fmt.Fprintf(os.Stderr, "  %s %s  %s\n",
				ui.Positive.Render("●"),
				ui.Positive.Bold(true).Render("Running"),
				ui.Muted.Render(fmt.Sprintf("PID %d, %s", pid, mode)),
			)
		} else {
			fmt.Fprintf(os.Stderr, "  %s %s\n",
				ui.Negative.Render("●"),
				ui.Muted.Render("Stopped"),
			)
		}

		fmt.Fprintln(os.Stderr)

		// Network info.
		fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Accent.Render("TLD"), ui.Bold.Render("."+settings.Defaults.TLD))
		fmt.Fprintf(os.Stderr, "  %s  %s  %s  %s\n",
			ui.Accent.Render("DNS"),
			fmt.Sprintf("127.0.0.1:%d", config.DNSPort),
			ui.Accent.Render("HTTPS"),
			":443",
		)

		// PHP version info.
		globalPHP := settings.Defaults.PHP
		versions, _ := phpenv.InstalledVersions()
		if len(versions) > 0 {
			var labels []string
			for _, v := range versions {
				if v == globalPHP {
					labels = append(labels, ui.Positive.Bold(true).Render(v)+" "+ui.Muted.Render("(default)"))
				} else {
					labels = append(labels, ui.Accent.Render(v))
				}
			}
			fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Accent.Render("PHP"), strings.Join(labels, ui.Muted.Render(" · ")))
		} else if globalPHP != "" {
			fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Accent.Render("PHP"), globalPHP)
		}

		// Sites.
		reg, err := registry.Load()
		if err != nil {
			fmt.Fprintf(os.Stderr, "\n  %s\n\n", ui.Muted.Render(fmt.Sprintf("Cannot load registry: %v", err)))
			return nil
		}

		projects := reg.List()

		if len(projects) == 0 {
			fmt.Fprintf(os.Stderr, "\n  %s\n", ui.Muted.Render("No sites linked. Run pv link in a project to get started."))
		} else {
			fmt.Fprintf(os.Stderr, "\n  %s  %s\n\n",
				ui.Accent.Render("Sites"),
				ui.Muted.Render(fmt.Sprintf("%d linked", len(projects))),
			)
			rows := make([][]string, len(projects))
			for i, p := range projects {
				phpV := p.PHP
				if phpV == "" {
					phpV = globalPHP
				}
				if phpV == "" {
					phpV = "-"
				}
				typeLabel := p.Type
				if typeLabel == "" {
					typeLabel = "unknown"
				}

				domain := "https://" + p.Name + "." + settings.Defaults.TLD
				rows[i] = []string{domain, typeLabel, phpV}
			}
			ui.Table([]string{"Site", "Type", "PHP"}, rows)
		}

		fmt.Fprintln(os.Stderr)

		return nil
	},
}

func init() {
	rootCmd.AddCommand(statusCmd)
}
