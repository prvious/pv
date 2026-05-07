// Package postgres holds cobra commands for the postgres:* / pg:* group.
package postgres

import (
	"fmt"
	"strings"

	pg "github.com/prvious/pv/internal/postgres"
)

// resolveMajor implements the disambiguation rule for commands taking an
// optional [major] argument:
//   - explicit arg: must be installed, returned verbatim.
//   - no arg + exactly one installed major: returns that major.
//   - no arg + zero installed: error suggesting `pv postgres:install`.
//   - no arg + multiple installed: error listing them.
func resolveMajor(args []string) (string, error) {
	installed, err := pg.InstalledMajors()
	if err != nil {
		return "", err
	}
	if len(args) > 0 {
		want := args[0]
		for _, m := range installed {
			if m == want {
				return want, nil
			}
		}
		return "", fmt.Errorf("postgres %s is not installed (run `pv postgres:install %s`)", want, want)
	}
	switch len(installed) {
	case 0:
		return "", fmt.Errorf("no postgres majors installed (run `pv postgres:install`)")
	case 1:
		return installed[0], nil
	default:
		return "", fmt.Errorf("multiple postgres majors installed (%s); specify which one", strings.Join(installed, ", "))
	}
}
