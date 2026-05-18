// Package state owns the per-service runtime-state file at
// ~/.pv/data/state.json. The file is top-level keyed by service name; each
// service's value is opaque JSON owned by that service's package, so two
// services cannot accidentally collide on the same key.
package state

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/config"
)

// State maps service-name → opaque JSON payload.
type State map[string]json.RawMessage

// Load reads ~/.pv/data/state.json. A missing file returns an empty State
// (no error). A corrupt file logs a warning to stderr and returns empty.
func Load() (State, error) {
	path := config.StatePath()
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return State{}, nil
		}
		return nil, fmt.Errorf("state: read %s: %w", path, err)
	}
	var s State
	if err := json.Unmarshal(data, &s); err != nil {
		fmt.Fprintf(os.Stderr, "state: %s is corrupt (%v); treating as empty\n", path, err)
		return State{}, nil
	}
	if s == nil {
		s = State{}
	}
	return s, nil
}

// Save writes s to ~/.pv/data/state.json atomically (temp file + rename).
func Save(s State) error {
	if err := config.EnsureDirs(); err != nil {
		return err
	}
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	path := config.StatePath()
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return fmt.Errorf("state: write tmp: %w", err)
	}
	if err := os.Rename(tmp, path); err != nil {
		if rmErr := os.Remove(tmp); rmErr != nil && !os.IsNotExist(rmErr) {
			fmt.Fprintf(os.Stderr, "state: cleanup tmp after rename failure: %v\n", rmErr)
		}
		return fmt.Errorf("state: rename: %w", err)
	}
	return nil
}
