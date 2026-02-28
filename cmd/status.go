package cmd

import (
	"fmt"

	"github.com/prvious/pv/internal/config"
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

		reg, err := registry.Load()
		if err != nil {
			fmt.Printf("Sites:   (cannot load registry: %v)\n", err)
		} else {
			fmt.Printf("Sites:   %d linked\n", len(reg.List()))
		}

		return nil
	},
}

func init() {
	rootCmd.AddCommand(statusCmd)
}
