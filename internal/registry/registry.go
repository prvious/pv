package registry

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
)

type ServiceInstance struct {
	Image       string `json:"image,omitempty"`
	Port        int    `json:"port"`
	ConsolePort int    `json:"console_port,omitempty"`
}

type ProjectServices struct {
	Mail     string `json:"mail,omitempty"`
	MySQL    string `json:"mysql,omitempty"`
	Postgres string `json:"postgres,omitempty"`
	Redis    string `json:"redis,omitempty"`
	S3       string `json:"s3,omitempty"`
}

type Project struct {
	Name string `json:"name"`
	Path string `json:"path"`
	Type string `json:"type"`
	PHP  string `json:"php,omitempty"`
	// Aliases are additional hostnames Caddy serves for this project.
	// Replaced wholesale from pv.yml on every link / relink — removing
	// an alias from pv.yml removes it from the registry on next link.
	Aliases   []string         `json:"aliases,omitempty"`
	Services  *ProjectServices `json:"services,omitempty"`
	Databases []string         `json:"databases,omitempty"`
}

type Registry struct {
	Services map[string]*ServiceInstance `json:"services,omitempty"`
	Projects []Project                   `json:"projects"`
}

func Load() (*Registry, error) {
	path := config.RegistryPath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Registry{Services: make(map[string]*ServiceInstance)}, nil
		}
		return nil, err
	}
	var reg Registry
	if err := json.Unmarshal(data, &reg); err != nil {
		return nil, err
	}
	if reg.Services == nil {
		reg.Services = make(map[string]*ServiceInstance)
	}
	return &reg, nil
}

func (r *Registry) Save() error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	data, err := json.MarshalIndent(r, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(config.RegistryPath(), data, 0644)
}

func (r *Registry) Add(p Project) error {
	if existing := r.Find(p.Name); existing != nil {
		return fmt.Errorf("project %q is already linked", p.Name)
	}
	r.Projects = append(r.Projects, p)
	return nil
}

func (r *Registry) UpdateWith(name string, fn func(*Project)) error {
	for i := range r.Projects {
		if r.Projects[i].Name == name {
			fn(&r.Projects[i])
			return nil
		}
	}
	return fmt.Errorf("project %q not found", name)
}

func (r *Registry) Remove(name string) error {
	for i, p := range r.Projects {
		if p.Name == name {
			r.Projects = append(r.Projects[:i], r.Projects[i+1:]...)
			return nil
		}
	}
	return fmt.Errorf("project %q not found", name)
}

func (r *Registry) Find(name string) *Project {
	for i := range r.Projects {
		if r.Projects[i].Name == name {
			return &r.Projects[i]
		}
	}
	return nil
}

func (r *Registry) FindByPath(path string) *Project {
	for i := range r.Projects {
		if r.Projects[i].Path == path {
			return &r.Projects[i]
		}
	}
	return nil
}

func (r *Registry) List() []Project {
	return r.Projects
}

func (r *Registry) AddService(key string, svc *ServiceInstance) error {
	if _, exists := r.Services[key]; exists {
		return fmt.Errorf("service %q is already added", key)
	}
	r.Services[key] = svc
	return nil
}

func (r *Registry) RemoveService(key string) error {
	if _, exists := r.Services[key]; !exists {
		return fmt.Errorf("service %q not found", key)
	}
	delete(r.Services, key)
	return nil
}

// FindService looks up a service by exact registry key.
func (r *Registry) FindService(key string) (*ServiceInstance, error) {
	return r.Services[key], nil
}

func (r *Registry) ListServices() map[string]*ServiceInstance {
	return r.Services
}

// ProjectsUsingService returns project names that reference a given service.
func (r *Registry) ProjectsUsingService(serviceName string) []string {
	var names []string
	for _, p := range r.Projects {
		if p.Services == nil {
			continue
		}
		switch serviceName {
		case "mail":
			if p.Services.Mail != "" {
				names = append(names, p.Name)
			}
		case "mysql":
			if p.Services.MySQL != "" {
				names = append(names, p.Name)
			}
		case "postgres":
			if p.Services.Postgres != "" {
				names = append(names, p.Name)
			}
		case "redis":
			if p.Services.Redis != "" {
				names = append(names, p.Name)
			}
		case "s3":
			if p.Services.S3 != "" {
				names = append(names, p.Name)
			}
		}
	}
	return names
}

// UnbindService removes a service binding from all projects.
func (r *Registry) UnbindService(serviceName string) {
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		switch serviceName {
		case "mail":
			r.Projects[i].Services.Mail = ""
		case "mysql":
			r.Projects[i].Services.MySQL = ""
		case "postgres":
			r.Projects[i].Services.Postgres = ""
		case "redis":
			r.Projects[i].Services.Redis = ""
		case "s3":
			r.Projects[i].Services.S3 = ""
		}
	}
}

// UnbindPostgresMajor clears Services.Postgres on every project bound to
// the given major. Projects bound to other majors are unaffected.
// Tighter than UnbindService("postgres") — that would clear all bindings
// regardless of major.
func (r *Registry) UnbindPostgresMajor(major string) {
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		if r.Projects[i].Services.Postgres == major {
			r.Projects[i].Services.Postgres = ""
		}
	}
}

// UnbindRedisVersion clears Services.Redis on every project bound to the
// given version. Projects bound to other versions are unaffected.
// Tighter than UnbindService("redis") — that would clear all redis bindings
// regardless of version, which is wrong when only one of several installed
// versions is being removed.
func (r *Registry) UnbindRedisVersion(version string) {
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		if r.Projects[i].Services.Redis == version {
			r.Projects[i].Services.Redis = ""
		}
	}
}

// UnbindMysqlVersion clears Services.MySQL on every project bound to the
// given version. Projects bound to other versions are unaffected.
// Tighter than UnbindService("mysql") — that would clear all mysql bindings
// regardless of version, which is wrong when only one of several installed
// versions is being removed.
func (r *Registry) UnbindMysqlVersion(version string) {
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		if r.Projects[i].Services.MySQL == version {
			r.Projects[i].Services.MySQL = ""
		}
	}
}

// UnbindMailVersion clears Services.Mail on every project bound to the
// given version. Projects bound to other versions are unaffected.
// Tighter than UnbindService("mail") — that would clear all mail bindings
// regardless of version, which is wrong when only one of several installed
// versions is being removed.
func (r *Registry) UnbindMailVersion(version string) {
	if version == "" {
		return
	}
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		if r.Projects[i].Services.Mail == version {
			r.Projects[i].Services.Mail = ""
		}
	}
}

// UnbindS3Version clears Services.S3 on every project bound to the
// given version. Projects bound to other versions are unaffected.
// Tighter than UnbindService("s3") — that would clear all s3 bindings
// regardless of version, which is wrong when only one of several installed
// versions is being removed.
func (r *Registry) UnbindS3Version(version string) {
	if version == "" {
		return
	}
	for i := range r.Projects {
		if r.Projects[i].Services == nil {
			continue
		}
		if r.Projects[i].Services.S3 == version {
			r.Projects[i].Services.S3 = ""
		}
	}
}

// GroupByPHP groups projects by their PHP version.
// Projects with an empty PHP field are grouped under the given defaultVersion.
func (r *Registry) GroupByPHP(defaultVersion string) map[string][]Project {
	groups := make(map[string][]Project)
	for _, p := range r.Projects {
		v := p.PHP
		if v == "" {
			v = defaultVersion
		}
		groups[v] = append(groups[v], p)
	}
	return groups
}
