package tools

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"

	"github.com/prvious/pv/internal/config"
)

// ExposureType determines how a tool is made available on the user's PATH.
type ExposureType int

const (
	ExposureNone    ExposureType = iota // never exposed automatically or manually
	ExposureSymlink                     // symlink internal/bin/X -> bin/X
	ExposureShim                        // custom bash shim script
)

// Tool describes a managed binary.
type Tool struct {
	Name        string       // set automatically from registry key
	DisplayName string
	AutoExpose  bool         // :install auto-calls :path
	Exposure    ExposureType
	// InternalPath returns where the real binary lives.
	InternalPath func() string
	// WriteShim writes a custom shim to ~/.pv/bin/<name>.
	// Only used when Exposure == ExposureShim.
	WriteShim func() error
}

// globalPHPVersion returns the global PHP version from settings.
func globalPHPVersion() string {
	s, err := config.LoadSettings()
	if err != nil || s.GlobalPHP == "" {
		return ""
	}
	return s.GlobalPHP
}

// registry of all managed tools, keyed by name.
var registry = map[string]*Tool{
	"php": {
		DisplayName: "PHP",
		AutoExpose:  true,
		Exposure:    ExposureShim,
		InternalPath: func() string {
			return filepath.Join(config.PhpVersionDir(globalPHPVersion()), "php")
		},
		WriteShim: writePhpShim,
	},
	"frankenphp": {
		DisplayName: "FrankenPHP",
		AutoExpose:  true,
		Exposure:    ExposureSymlink,
		InternalPath: func() string {
			return filepath.Join(config.PhpVersionDir(globalPHPVersion()), "frankenphp")
		},
	},
	"composer": {
		DisplayName: "Composer",
		AutoExpose:  true,
		Exposure:    ExposureSymlink,
		InternalPath: func() string {
			return config.ComposerPharPath()
		},
	},
	"mago": {
		DisplayName: "Mago",
		AutoExpose:  true,
		Exposure:    ExposureSymlink,
		InternalPath: func() string {
			return config.MagoPath()
		},
	},
	"colima": {
		DisplayName: "Colima",
		AutoExpose:  false,
		Exposure:    ExposureSymlink,
		InternalPath: func() string {
			return config.ColimaPath()
		},
	},
}

func init() {
	// Derive Name from map key so they can never diverge.
	for name, t := range registry {
		t.Name = name
	}
}

// Get returns the tool with the given name, or nil.
func Get(name string) *Tool {
	return registry[name]
}

// MustGet returns the tool with the given name, or panics.
// Use for compile-time constant names where a missing tool is a programming error.
func MustGet(name string) *Tool {
	t := registry[name]
	if t == nil {
		panic(fmt.Sprintf("tools: unknown tool %q", name))
	}
	return t
}

// List returns all tools sorted by name.
func List() []*Tool {
	var out []*Tool
	for _, t := range registry {
		out = append(out, t)
	}
	sort.Slice(out, func(i, j int) bool {
		return out[i].Name < out[j].Name
	})
	return out
}

// Expose creates the shim or symlink in ~/.pv/bin/ for a tool.
func Expose(t *Tool) error {
	binDir := config.BinDir()

	switch t.Exposure {
	case ExposureNone:
		return nil
	case ExposureShim:
		if t.WriteShim == nil {
			return fmt.Errorf("tool %s has ExposureShim but no WriteShim func", t.Name)
		}
		return t.WriteShim()
	case ExposureSymlink:
		target := t.InternalPath()
		linkPath := filepath.Join(binDir, t.Name)
		if err := os.Remove(linkPath); err != nil && !os.IsNotExist(err) {
			return fmt.Errorf("cannot remove existing %s: %w", linkPath, err)
		}
		if err := os.Symlink(target, linkPath); err != nil {
			return fmt.Errorf("cannot create symlink %s -> %s: %w", linkPath, target, err)
		}
		return nil
	default:
		return fmt.Errorf("unknown exposure type for tool %s", t.Name)
	}
}

// Unexpose removes the shim or symlink from ~/.pv/bin/.
func Unexpose(t *Tool) error {
	if t.Exposure == ExposureNone {
		return nil
	}
	linkPath := filepath.Join(config.BinDir(), t.Name)
	if err := os.Remove(linkPath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("cannot remove %s: %w", linkPath, err)
	}
	return nil
}

// IsExposed checks if a tool is currently on PATH (exists in ~/.pv/bin/).
func IsExposed(t *Tool) bool {
	linkPath := filepath.Join(config.BinDir(), t.Name)
	_, err := os.Lstat(linkPath)
	return err == nil
}

// ExposeAll exposes all tools that have AutoExpose=true.
func ExposeAll() error {
	for _, t := range registry {
		if !t.AutoExpose {
			continue
		}
		if err := Expose(t); err != nil {
			return fmt.Errorf("cannot expose %s: %w", t.Name, err)
		}
	}
	return nil
}
