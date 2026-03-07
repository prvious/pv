package cmd

import "github.com/prvious/pv/internal/commands/colima"

func init() {
	colima.Register(rootCmd)
}
