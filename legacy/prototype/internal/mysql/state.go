package mysql

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/state"
)

const stateKey = "mysql"

// Wanted-state values for VersionState.Wanted. Bare strings would let typos
// silently persist (and be silently read as "not running"), so callers go
// through SetWanted which validates against this set.
const (
	WantedRunning = "running"
	WantedStopped = "stopped"
)

// VersionState is the per-version sub-record of mysql state.
type VersionState struct {
	Wanted string `json:"wanted"`
}

// State is the mysql slice of ~/.pv/data/state.json.
//
// Note the JSON tag uses "versions" (matching the spec) rather than the
// postgres package's "majors" — mysql's identifier is a major.minor pair.
type State struct {
	Versions map[string]VersionState `json:"versions"`
}

// LoadState reads the mysql slice. Missing or empty → zero-value state.
// A corrupt slice is treated as empty with a one-time stderr warning, the
// same posture postgres takes — the recovery path is `mysql:start <version>`.
func LoadState() (State, error) {
	all, err := state.Load()
	if err != nil {
		return State{Versions: map[string]VersionState{}}, err
	}
	raw, ok := all[stateKey]
	if !ok {
		return State{Versions: map[string]VersionState{}}, nil
	}
	var s State
	if err := json.Unmarshal(raw, &s); err != nil {
		fmt.Fprintf(os.Stderr, "mysql: state slice corrupt (%v); treating as empty\n", err)
		return State{Versions: map[string]VersionState{}}, nil
	}
	if s.Versions == nil {
		s.Versions = map[string]VersionState{}
	}
	return s, nil
}

// SaveState writes the mysql slice, preserving other services' slices.
func SaveState(s State) error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	if s.Versions == nil {
		s.Versions = map[string]VersionState{}
	}
	payload, err := json.Marshal(s)
	if err != nil {
		return err
	}
	all[stateKey] = payload
	return state.Save(all)
}

// SetWanted updates the wanted-state for one version and persists.
// Rejects values outside the WantedRunning/WantedStopped set so a typo
// can't silently persist garbage that WantedVersions will later read as
// "not running" (and stop the process).
func SetWanted(version, wanted string) error {
	if wanted != WantedRunning && wanted != WantedStopped {
		return fmt.Errorf("mysql: invalid wanted state %q (want %q or %q)", wanted, WantedRunning, WantedStopped)
	}
	s, err := LoadState()
	if err != nil {
		return err
	}
	s.Versions[version] = VersionState{Wanted: wanted}
	return SaveState(s)
}

// RemoveVersion drops a version's entry from state and persists.
func RemoveVersion(version string) error {
	s, err := LoadState()
	if err != nil {
		return err
	}
	delete(s.Versions, version)
	return SaveState(s)
}
