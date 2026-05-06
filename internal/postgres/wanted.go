package postgres

import (
	"fmt"
	"os"
	"sort"
)

// WantedMajors returns the majors that should currently be supervised:
// majors marked wanted="running" in state.json AND installed on disk.
// Stale entries (state says running but binaries are missing) emit a
// stderr warning and are filtered out.
func WantedMajors() ([]string, error) {
	st, err := LoadState()
	if err != nil {
		return nil, err
	}
	installed, err := InstalledMajors()
	if err != nil {
		return nil, err
	}
	installedSet := map[string]struct{}{}
	for _, m := range installed {
		installedSet[m] = struct{}{}
	}
	var out []string
	for major, ms := range st.Majors {
		if ms.Wanted != "running" {
			continue
		}
		if _, ok := installedSet[major]; !ok {
			fmt.Fprintf(os.Stderr, "postgres: state.json wants %s running but binaries are missing; skipping\n", major)
			continue
		}
		out = append(out, major)
	}
	sort.Strings(out)
	return out, nil
}
