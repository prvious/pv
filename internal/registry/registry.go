package registry

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
)

type Project struct {
	Name string `json:"name"`
	Path string `json:"path"`
	Type string `json:"type"`
	PHP  string `json:"php,omitempty"`
}

type Registry struct {
	Projects []Project `json:"projects"`
}

func Load() (*Registry, error) {
	path := config.RegistryPath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Registry{}, nil
		}
		return nil, err
	}
	var reg Registry
	if err := json.Unmarshal(data, &reg); err != nil {
		return nil, err
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
	for _, p := range r.Projects {
		if p.Name == name {
			return &p
		}
	}
	return nil
}

func (r *Registry) FindByPath(path string) *Project {
	for _, p := range r.Projects {
		if p.Path == path {
			return &p
		}
	}
	return nil
}

func (r *Registry) List() []Project {
	return r.Projects
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
