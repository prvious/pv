package cmd

import "github.com/prvious/pv/internal/commands/service"

func init() {
	service.Register(rootCmd)
}
