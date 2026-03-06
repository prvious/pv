package colima

import (
	"github.com/prvious/pv/internal/registry"
)

// RecoverServices checks all registered services and ensures their containers
// are running. This is called on daemon start after Colima is verified running.
// The actual Docker operations are performed by the caller since we don't import
// the container package here to avoid circular dependencies.
func ServicesToRecover(reg *registry.Registry) []string {
	var keys []string
	for key := range reg.Services {
		keys = append(keys, key)
	}
	return keys
}
