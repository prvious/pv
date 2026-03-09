package config

import (
	"encoding/json"
	"fmt"
	"os"
	"regexp"

	"gopkg.in/yaml.v3"
)

type Defaults struct {
	PHP string `yaml:"php,omitempty"`
	TLD string `yaml:"tld"`
}

type Settings struct {
	Defaults Defaults `yaml:"defaults"`
}

var validTLD = regexp.MustCompile(`^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$`)

func DefaultSettings() *Settings {
	return &Settings{Defaults: Defaults{TLD: "test"}}
}

func LoadSettings() (*Settings, error) {
	path := SettingsPath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return migrateOrDefault()
		}
		return nil, err
	}
	var s Settings
	if err := yaml.Unmarshal(data, &s); err != nil {
		return nil, err
	}
	if s.Defaults.TLD == "" {
		s.Defaults.TLD = "test"
	}
	return &s, nil
}

// migrateOrDefault migrates from the old settings.json if it exists,
// otherwise returns default settings.
func migrateOrDefault() (*Settings, error) {
	old := oldSettingsPath()
	data, err := os.ReadFile(old)
	if err != nil {
		return DefaultSettings(), nil
	}

	var legacy struct {
		TLD       string `json:"tld"`
		GlobalPHP string `json:"global_php,omitempty"`
	}
	if err := json.Unmarshal(data, &legacy); err != nil {
		return DefaultSettings(), nil
	}

	s := &Settings{
		Defaults: Defaults{
			PHP: legacy.GlobalPHP,
			TLD: legacy.TLD,
		},
	}
	if s.Defaults.TLD == "" {
		s.Defaults.TLD = "test"
	}

	if err := s.Save(); err != nil {
		return s, nil
	}
	os.Remove(old)
	return s, nil
}

func (s *Settings) Save() error {
	if err := EnsureDirs(); err != nil {
		return err
	}
	data, err := yaml.Marshal(s)
	if err != nil {
		return err
	}
	return os.WriteFile(SettingsPath(), data, 0644)
}

func ValidateTLD(tld string) error {
	if tld == "" {
		return fmt.Errorf("TLD cannot be empty")
	}
	if !validTLD.MatchString(tld) {
		return fmt.Errorf("invalid TLD %q: must be 1-63 lowercase alphanumeric characters or hyphens, cannot start or end with a hyphen", tld)
	}
	return nil
}
