package cmd

import (
	"github.com/prvious/pv/internal/commands/colima"
	"github.com/prvious/pv/internal/commands/service"
)

func init() {
	colima.StopServicesFunc = service.RunStop
	colima.Register(rootCmd)
}
