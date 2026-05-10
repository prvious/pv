package rustfs

import "github.com/prvious/pv/internal/registry"

// BindToAllProjects sets Services.S3 = true on every Laravel project so
// that UpdateLinkedProjectsEnv can find projects that were linked
// before the service existed. Caller is responsible for saving the
// registry.
func BindToAllProjects(reg *registry.Registry) error {
	for i := range reg.Projects {
		p := &reg.Projects[i]
		if p.Type != "laravel" && p.Type != "laravel-octane" {
			continue
		}
		if p.Services == nil {
			p.Services = &registry.ProjectServices{}
		}
		p.Services.S3 = true
	}
	return nil
}
