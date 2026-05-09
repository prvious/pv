// Package mysql holds cobra commands for the mysql:* group. There is
// intentionally no alias namespace (no my:*) — the mysql: prefix is
// already short enough.
package mysql

import (
	"fmt"
	"strings"

	my "github.com/prvious/pv/internal/mysql"
)

// ResolveVersion implements the disambiguation rule for commands taking an
// optional [version] argument:
//   - explicit arg: must be installed, returned verbatim.
//   - no arg + exactly one installed version: returns that version.
//   - no arg + zero installed: error suggesting `pv mysql:install`.
//   - no arg + multiple installed: error listing them.
//
// Exported so orchestrators (`pv update`, `pv uninstall`) can reuse the
// same rule without re-implementing it.
func ResolveVersion(args []string) (string, error) {
	installed, err := my.InstalledVersions()
	if err != nil {
		return "", err
	}
	if len(args) > 0 {
		want := args[0]
		for _, v := range installed {
			if v == want {
				return want, nil
			}
		}
		return "", fmt.Errorf("mysql %s is not installed (run `pv mysql:install %s`)", want, want)
	}
	switch len(installed) {
	case 0:
		return "", fmt.Errorf("no mysql versions installed (run `pv mysql:install`)")
	case 1:
		return installed[0], nil
	default:
		return "", fmt.Errorf("multiple mysql versions installed (%s); specify which one", strings.Join(installed, ", "))
	}
}
