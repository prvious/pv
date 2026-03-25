package registry

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/prvious/pv/internal/config"
)

type ServiceInstance struct {
	Image       string `json:"image"`
	Port        int    `json:"port"`
	ConsolePort int    `json:"console_port,omitempty"`
}

type ProjectServices struct {
	Mail     bool   `json:"mail,omitempty"`
	MySQL    string `json:"mysql,omitempty"`
	Postgres string `json:"postgres,omitempty"`
	Redis    bool   `json:"redis,omitempty"`
	S3       bool   `json:"s3,omitempty"`
}

type Project struct {
	Name      string           `json:"name"`
	Path      string           `json:"path"`
	Type      string           `json:"type"`
	PHP       string           `json:"php,omitempty"`
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

// FindService looks up a service by exact key first (e.g. "mysql:8.4"),
// then falls back to matching by service name prefix (e.g. "mysql" matches "mysql:8.4").
func (r *Registry) FindService(key string) *ServiceInstance {
	if svc, ok := r.Services[key]; ok {
		return svc
	}
	// Fuzzy match: "postgres" matches "postgres:18-alpine".
	for k, svc := range r.Services {
		if strings.HasPrefix(k, key+":") || k == key {
			return svc
		}
	}
	return nil
}

// ResolveServiceKey returns the full registry key for a service, supporting
// both exact keys ("mysql:8.4") and name-only lookups ("mysql").
func (r *Registry) ResolveServiceKey(key string) string {
	if _, ok := r.Services[key]; ok {
		return key
	}
	for k := range r.Services {
		if strings.HasPrefix(k, key+":") || k == key {
			return k
		}
	}
	return key
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
			if p.Services.Mail {
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
			if p.Services.Redis {
				names = append(names, p.Name)
			}
		case "s3":
			if p.Services.S3 {
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
			r.Projects[i].Services.Mail = false
		case "mysql":
			r.Projects[i].Services.MySQL = ""
		case "postgres":
			r.Projects[i].Services.Postgres = ""
		case "redis":
			r.Projects[i].Services.Redis = false
		case "s3":
			r.Projects[i].Services.S3 = false
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
