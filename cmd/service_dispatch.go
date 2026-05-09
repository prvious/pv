package cmd

import (
	"fmt"

	mailpit "github.com/prvious/pv/internal/commands/mailpit"
	rustfs "github.com/prvious/pv/internal/commands/rustfs"
	"github.com/prvious/pv/internal/commands/service"
	"github.com/prvious/pv/internal/services"
)

// addService dispatches a service-add request to the right entrypoint:
// binary services (s3 / mail) now route to their first-class commands
// (rustfs:install / mailpit:install) since service:* no longer accepts
// them; docker services continue through service:add as before.
//
// Used by the install and setup orchestrators that take a flat list of
// service names from the CLI / wizard and need to materialize each one.
func addService(name, version string) error {
	kind, _, _, err := services.LookupAny(name)
	if err != nil {
		return err
	}
	switch kind {
	case services.KindBinary:
		switch name {
		case "s3":
			return rustfs.RunInstall()
		case "mail":
			return mailpit.RunInstall()
		default:
			return fmt.Errorf("unknown binary service %q (no first-class command)", name)
		}
	case services.KindDocker:
		args := []string{name}
		if version != "" {
			args = append(args, version)
		}
		return service.RunAdd(args)
	}
	return fmt.Errorf("unknown service %q", name)
}
