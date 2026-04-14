package colima

import (
	"sort"

	"github.com/prvious/pv/internal/registry"
)

// ServicesToRecover returns the registry keys of docker-backed services that
// the caller should ensure are running. Binary-backed services are handled
// by the supervisor inside the daemon and must not be recovered here.
func ServicesToRecover(reg *registry.Registry) []string {
	var keys []string
	for key, inst := range reg.Services {
		if inst.Kind == "binary" {
			continue
		}
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}
