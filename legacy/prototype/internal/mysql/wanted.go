package mysql

import (
	"fmt"
	"os"
	"sort"
)

// WantedVersions returns the versions that should currently be supervised:
// versions marked wanted="running" in state.json AND installed on disk.
// Stale entries (state says running but binaries are missing) emit a
// stderr warning and are filtered out — recovery is `mysql:install` or
// `mysql:start` after the binaries are restored.
func WantedVersions() ([]string, error) {
	st, err := LoadState()
	if err != nil {
		return nil, err
	}
	installed, err := InstalledVersions()
	if err != nil {
		return nil, err
	}
	installedSet := map[string]struct{}{}
	for _, v := range installed {
		installedSet[v] = struct{}{}
	}
	var out []string
	for version, vs := range st.Versions {
		if vs.Wanted != WantedRunning {
			continue
		}
		if _, ok := installedSet[version]; !ok {
			fmt.Fprintf(os.Stderr, "mysql: state.json wants %s running but binaries are missing; skipping\n", version)
			continue
		}
		out = append(out, version)
	}
	sort.Strings(out)
	return out, nil
}
