package cmd

import (
	"fmt"
	"strings"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/phpenv"
	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/server"
	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:   "status",
	Short: "Show pv server status",
	RunE: func(cmd *cobra.Command, args []string) error {
		settings, err := config.LoadSettings()
		if err != nil {
			return fmt.Errorf("cannot load settings: %w", err)
		}

		running := server.IsRunning()
		if running {
			pid, _ := server.ReadPID()
			fmt.Printf("Status:  running (PID %d)\n", pid)
		} else {
			fmt.Println("Status:  stopped")
		}

		fmt.Printf("TLD:     .%s\n", settings.TLD)
		fmt.Printf("DNS:     127.0.0.1:%d\n", config.DNSPort)
		fmt.Println("HTTPS:   port 443")
		fmt.Println("HTTP:    port 80")

		// PHP version info.
		globalPHP := settings.GlobalPHP
		if globalPHP != "" {
			fmt.Printf("PHP:     %s (global)\n", globalPHP)
		}

		versions, _ := phpenv.InstalledVersions()
		if len(versions) > 0 {
			var labels []string
			for _, v := range versions {
				if v == globalPHP {
					labels = append(labels, v+"*")
				} else {
					labels = append(labels, v)
				}
			}
			fmt.Printf("PHP installed: %s\n", strings.Join(labels, ", "))
		}

		reg, err := registry.Load()
		if err != nil {
			fmt.Printf("Sites:   (cannot load registry: %v)\n", err)
			return nil
		}

		projects := reg.List()
		fmt.Printf("Sites:   %d linked\n", len(projects))

		if len(projects) > 0 && running {
			fmt.Println()
			fmt.Println("Projects:")
			for _, p := range projects {
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
				portInfo := ""
				if phpV != globalPHP && phpV != "" && phpV != "-" {
					port := config.PortForVersion(phpV)
					portInfo = fmt.Sprintf(" (port %d)", port)
				}
				fmt.Printf("  %-20s %-16s PHP %s%s\n", p.Name+"."+settings.TLD, typeLabel, phpV, portInfo)
			}
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(statusCmd)
}
