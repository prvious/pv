package redis

import (
	"fmt"
	"os"
	"sort"
)

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
			fmt.Fprintf(os.Stderr, "redis: state.json wants %s running but binary is missing; skipping\n", version)
			continue
		}
		out = append(out, version)
	}
	sort.Strings(out)
	return out, nil
}
