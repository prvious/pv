package services

import (
	"fmt"
	"strings"
)

// Kind classifies which registry a service name resolves to.
type Kind int

const (
	KindUnknown Kind = iota
	KindDocker
	KindBinary
)

// LookupAny resolves a service name across both the Docker and binary
// registries.
//
// Lookup order is binary first, then Docker. A name found in only one
// registry returns that kind. A name in neither registry returns
// KindUnknown plus a non-nil error whose text matches the Lookup error
// format so callsites switching from Lookup retain the same error UX.
//
// When kind != KindUnknown, exactly one of binSvc / docSvc is non-nil.
func LookupAny(name string) (kind Kind, binSvc BinaryService, docSvc Service, err error) {
	if svc, ok := LookupBinary(name); ok {
		return KindBinary, svc, nil, nil
	}
	if svc, lookupErr := Lookup(name); lookupErr == nil {
		return KindDocker, nil, svc, nil
	}
	return KindUnknown, nil, nil, fmt.Errorf(
		"unknown service %q (available: %s)",
		name,
		strings.Join(Available(), ", "),
	)
}
