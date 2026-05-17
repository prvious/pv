package installer

import (
	"errors"
	"fmt"
	"sort"
)

type Kind string

const (
	KindRuntime Kind = "runtime"
	KindTool    Kind = "tool"
	KindService Kind = "service"
)

type Identity struct {
	Kind    Kind
	Name    string
	Version string
}

func (id Identity) String() string {
	return string(id.Kind) + ":" + id.Name + ":" + id.Version
}

type Item struct {
	ID        Identity
	DependsOn []Identity
}

type Plan struct {
	Items []Item
}

func (p Plan) Validate() error {
	seen := make(map[string]Identity, len(p.Items))
	for _, item := range p.Items {
		if err := validateIdentity(item.ID); err != nil {
			return err
		}
		key := item.ID.String()
		if existing, ok := seen[key]; ok {
			return fmt.Errorf("duplicate install item %s", existing)
		}
		seen[key] = item.ID
	}
	for _, item := range p.Items {
		for _, dependency := range item.DependsOn {
			if err := validateIdentity(dependency); err != nil {
				return err
			}
			if _, ok := seen[dependency.String()]; !ok {
				return fmt.Errorf("install item %s depends on missing item %s", item.ID, dependency)
			}
		}
	}
	if _, err := p.Order(); err != nil {
		return err
	}
	return nil
}

func (p Plan) Order() ([]Item, error) {
	items := make(map[string]Item, len(p.Items))
	dependents := make(map[string][]string, len(p.Items))
	remaining := make(map[string]int, len(p.Items))
	for _, item := range p.Items {
		key := item.ID.String()
		items[key] = item
		remaining[key] = len(item.DependsOn)
		for _, dependency := range item.DependsOn {
			dependencyKey := dependency.String()
			dependents[dependencyKey] = append(dependents[dependencyKey], key)
		}
	}

	var ready []string
	for key, count := range remaining {
		if count == 0 {
			ready = append(ready, key)
		}
	}
	sort.Strings(ready)

	ordered := make([]Item, 0, len(p.Items))
	for len(ready) > 0 {
		key := ready[0]
		ready = ready[1:]
		ordered = append(ordered, items[key])
		for _, dependent := range dependents[key] {
			remaining[dependent]--
			if remaining[dependent] == 0 {
				ready = append(ready, dependent)
				sort.Strings(ready)
			}
		}
		delete(remaining, key)
	}

	if len(ordered) != len(p.Items) {
		return nil, errors.New("install plan contains a dependency cycle")
	}
	return ordered, nil
}

func validateIdentity(id Identity) error {
	switch id.Kind {
	case KindRuntime, KindTool, KindService:
	default:
		return fmt.Errorf("invalid install item kind %q", id.Kind)
	}
	if id.Name == "" {
		return errors.New("install item name is required")
	}
	if id.Version == "" {
		return errors.New("install item version is required")
	}
	return nil
}
