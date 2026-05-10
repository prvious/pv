package services

import (
	"fmt"
	"sort"
	"strings"
)

// WebRoute maps a subdomain under pv.{tld} to a local port.
// For example, {Subdomain: "s3", Port: 9001} routes s3.pv.test → 127.0.0.1:9001.
type WebRoute struct {
	Subdomain string
	Port      int
}

// Available returns the names of all registered services, sorted.
// All services now run as native binaries supervised by the daemon.
func Available() []string {
	names := make([]string, 0, len(binaryRegistry))
	for n := range binaryRegistry {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}

// Lookup returns the BinaryService registered under name, or an error
// listing the available services.
func Lookup(name string) (BinaryService, error) {
	if svc, ok := binaryRegistry[name]; ok {
		return svc, nil
	}
	return nil, fmt.Errorf("unknown service %q (available: %s)", name, strings.Join(Available(), ", "))
}
