package rustfs

import "github.com/prvious/pv/internal/registry"

// BindToAllProjects sets Services.S3 = true on every Laravel project in
// the registry so the service is reachable from projects that were linked
// before the service existed. Caller is responsible for saving the
// registry. Does not touch project .env files — that's pv link's job.
func BindToAllProjects(reg *registry.Registry) {
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
}
