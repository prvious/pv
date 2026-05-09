// Package service holds cobra commands for the service:* group, which
// now manages docker-backed services (mysql, redis, ...) only. The
// formerly-binary services s3 (RustFS) and mail (Mailpit) live under
// their own first-class command groups: rustfs:* / mailpit:* with
// s3:* / mail:* aliases.
package service

import (
	"fmt"

	"github.com/prvious/pv/internal/services"
)

// redirectIfBinary returns a redirect error when name resolves to a
// binary service in the global registry. The action verb tailors the
// suggestion ("install", "start", "uninstall", ...) so the user gets
// a directly-runnable next step. Returns nil for docker / unknown
// names — those flow through to services.Lookup as before.
func redirectIfBinary(name, action string) error {
	binSvc, ok := services.LookupBinary(name)
	if !ok {
		return nil
	}
	return fmt.Errorf(
		"%s is now a first-class command — use `pv %s:%s` (alias: `pv %s:%s`)",
		name, binSvc.Binary().Name, action, name, action,
	)
}
