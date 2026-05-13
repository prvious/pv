package mailpit

import (
	"fmt"

	pkg "github.com/prvious/pv/internal/mailpit"
	"github.com/prvious/pv/internal/server"
	"github.com/prvious/pv/internal/ui"
)

func signalDaemon(serviceName string) error {
	if !server.IsRunning() {
		ui.Subtle(fmt.Sprintf("daemon not running - %s will start on next `pv start`", serviceName))
		return nil
	}
	return server.SignalDaemon()
}

func argVersion(args []string) string {
	if len(args) > 0 {
		return args[0]
	}
	return ""
}

func printConnectionDetails(version string) {
	ui.Subtle(fmt.Sprintf("Web UI: http://127.0.0.1:%d", pkg.ConsolePort()))
	ui.Subtle(fmt.Sprintf("SMTP:   127.0.0.1:%d", pkg.Port()))
}
