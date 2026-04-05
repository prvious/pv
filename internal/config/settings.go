package config

import (
	"fmt"
	"os"
	"regexp"

	"gopkg.in/yaml.v3"
)

// AutoMode controls whether an automation step runs automatically.
type AutoMode string

const (
	AutoOn  AutoMode = "true"
	AutoOff AutoMode = "false"
	AutoAsk AutoMode = "ask"
)

// VMConfig holds Colima VM resource allocation settings.
type VMConfig struct {
	CPU    int `yaml:"cpu,omitempty"`
	Memory int `yaml:"memory,omitempty"`
	Disk   int `yaml:"disk,omitempty"`
}

// DefaultVMConfig returns sensible defaults for the Colima VM.
func DefaultVMConfig() VMConfig {
	return VMConfig{CPU: 2, Memory: 2, Disk: 60}
}

// WithDefaults returns a copy with zero values replaced by defaults.
func (vm VMConfig) WithDefaults() VMConfig {
	d := DefaultVMConfig()
	if vm.CPU <= 0 {
		vm.CPU = d.CPU
	}
	if vm.Memory <= 0 {
		vm.Memory = d.Memory
	}
	if vm.Disk <= 0 {
		vm.Disk = d.Disk
	}
	return vm
}

type Defaults struct {
	PHP    string   `yaml:"php,omitempty"`
	TLD    string   `yaml:"tld"`
	Daemon *bool    `yaml:"daemon,omitempty"`
	VM     VMConfig `yaml:"vm,omitempty"`
}

// Automation controls which link-time steps run automatically.
type Automation struct {
	InstallPHPVersion AutoMode `yaml:"install_php_version,omitempty"`
	ComposerInstall   AutoMode `yaml:"composer_install,omitempty"`
	CopyEnv           AutoMode `yaml:"copy_env,omitempty"`
	GenerateKey       AutoMode `yaml:"generate_key,omitempty"`
	SetAppURL         AutoMode `yaml:"set_app_url,omitempty"`
	SetViteTLS        AutoMode `yaml:"set_vite_tls,omitempty"`
	InstallOctane     AutoMode `yaml:"install_octane,omitempty"`
	CreateDatabase    AutoMode `yaml:"create_database,omitempty"`
	RunMigrations     AutoMode `yaml:"run_migrations,omitempty"`
	ServiceEnvUpdate  AutoMode `yaml:"update_env_on_service,omitempty"`
	ServiceFallback   AutoMode `yaml:"service_fallback,omitempty"`

	// Hidden gates — not shown in setup wizard, configurable via pv.yml.
	GenerateSiteConfig AutoMode `yaml:"generate_site_config,omitempty"`
	GenerateCaddyfile  AutoMode `yaml:"generate_caddyfile,omitempty"`
	GenerateTLSCert    AutoMode `yaml:"generate_tls_cert,omitempty"`
	DetectServices     AutoMode `yaml:"detect_services,omitempty"`
}

type Settings struct {
	Defaults   Defaults   `yaml:"defaults"`
	Automation Automation `yaml:"automation,omitempty"`
}

var validTLD = regexp.MustCompile(`^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$`)

// DefaultAutomation returns the default automation settings.
func DefaultAutomation() Automation {
	return Automation{
		InstallPHPVersion:  AutoOn,
		ComposerInstall:    AutoOn,
		CopyEnv:            AutoOn,
		GenerateKey:        AutoOn,
		SetAppURL:          AutoOn,
		SetViteTLS:         AutoOn,
		InstallOctane:      AutoAsk,
		CreateDatabase:     AutoOn,
		RunMigrations:      AutoAsk,
		ServiceEnvUpdate:   AutoOn,
		ServiceFallback:    AutoOn,
		GenerateSiteConfig: AutoOn,
		GenerateCaddyfile:  AutoOn,
		GenerateTLSCert:    AutoOn,
		DetectServices:     AutoOn,
	}
}

func validAutoMode(m AutoMode) bool {
	return m == AutoOn || m == AutoOff || m == AutoAsk
}

// applyAutomationDefaults fills empty Automation fields with defaults
// and replaces invalid values with the default.
func applyAutomationDefaults(a *Automation) {
	d := DefaultAutomation()
	if !validAutoMode(a.InstallPHPVersion) {
		a.InstallPHPVersion = d.InstallPHPVersion
	}
	if !validAutoMode(a.ComposerInstall) {
		a.ComposerInstall = d.ComposerInstall
	}
	if !validAutoMode(a.CopyEnv) {
		a.CopyEnv = d.CopyEnv
	}
	if !validAutoMode(a.GenerateKey) {
		a.GenerateKey = d.GenerateKey
	}
	if !validAutoMode(a.SetAppURL) {
		a.SetAppURL = d.SetAppURL
	}
	if !validAutoMode(a.SetViteTLS) {
		a.SetViteTLS = d.SetViteTLS
	}
	if !validAutoMode(a.InstallOctane) {
		a.InstallOctane = d.InstallOctane
	}
	if !validAutoMode(a.CreateDatabase) {
		a.CreateDatabase = d.CreateDatabase
	}
	if !validAutoMode(a.RunMigrations) {
		a.RunMigrations = d.RunMigrations
	}
	if !validAutoMode(a.ServiceEnvUpdate) {
		a.ServiceEnvUpdate = d.ServiceEnvUpdate
	}
	if !validAutoMode(a.ServiceFallback) {
		a.ServiceFallback = d.ServiceFallback
	}
	if !validAutoMode(a.GenerateSiteConfig) {
		a.GenerateSiteConfig = d.GenerateSiteConfig
	}
	if !validAutoMode(a.GenerateCaddyfile) {
		a.GenerateCaddyfile = d.GenerateCaddyfile
	}
	if !validAutoMode(a.GenerateTLSCert) {
		a.GenerateTLSCert = d.GenerateTLSCert
	}
	if !validAutoMode(a.DetectServices) {
		a.DetectServices = d.DetectServices
	}
}

// DaemonEnabled returns whether the daemon should be enabled.
// Defaults to true when not set.
func (d Defaults) DaemonEnabled() bool {
	if d.Daemon == nil {
		return true
	}
	return *d.Daemon
}

// BoolPtr returns a pointer to a bool value.
func BoolPtr(b bool) *bool { return &b }

func DefaultSettings() *Settings {
	return &Settings{
		Defaults:   Defaults{TLD: "test", Daemon: BoolPtr(true)},
		Automation: DefaultAutomation(),
	}
}

func LoadSettings() (*Settings, error) {
	path := SettingsPath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return DefaultSettings(), nil
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
	if s.Defaults.Daemon == nil {
		s.Defaults.Daemon = BoolPtr(true)
	}
	applyAutomationDefaults(&s.Automation)
	return &s, nil
}

func (s *Settings) Save() error {
	if s.Defaults.TLD == "" {
		s.Defaults.TLD = "test"
	}
	if err := ValidateTLD(s.Defaults.TLD); err != nil {
		return err
	}
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
