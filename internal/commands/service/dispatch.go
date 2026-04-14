package service

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

type serviceKind int

const (
	kindUnknown serviceKind = iota
	kindDocker
	kindBinary
)

// resolveKind determines whether the named service is a binary or docker
// service, returning at most one of the concrete service values.
// If the name matches a binary service but the registry already holds a
// docker-shaped entry for that name, an error is returned: no silent
// auto-migration. The user's remedy is `pv uninstall && pv setup`.
func resolveKind(reg *registry.Registry, name string) (serviceKind, services.BinaryService, services.Service, error) {
	binSvc, binOK := services.LookupBinary(name)
	docSvc, docErr := services.Lookup(name)

	if binOK {
		// Guard against a pre-existing docker-shaped entry for what is now
		// a binary service.
		if existing, ok := reg.Services[name]; ok {
			if existing.Kind != "binary" {
				return kindUnknown, nil, nil, fmt.Errorf(
					"%s is already registered (as docker) from a previous pv version. "+
						"Run `pv uninstall && pv setup` to reset", name)
			}
		}
		return kindBinary, binSvc, nil, nil
	}
	if docErr == nil {
		return kindDocker, nil, docSvc, nil
	}
	return kindUnknown, nil, nil, docErr
}
