package redis

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/state"
)

const stateKey = "redis"

// Wanted-state values for State.Wanted. Bare strings would let typos
// silently persist (and be silently read as "not running"), so callers
// go through SetWanted which validates against this set.
const (
	WantedRunning = "running"
	WantedStopped = "stopped"
)

// State is the redis slice of ~/.pv/data/state.json.
//
// Note the shape is FLAT (no Versions map) — redis is single-version, so
// a per-version sub-record would just add a layer of indirection over a
// single record. Compare with internal/mysql/state.go which uses a
// Versions map to disambiguate 8.0/8.4/9.7.
//
// On-disk JSON shape:
//
//	{
//	  "redis": { "wanted": "running" }
//	}
type State struct {
	Wanted string `json:"wanted"`
}

// LoadState reads the redis slice. Missing or empty → zero-value state.
// A corrupt slice is treated as empty with a one-time stderr warning,
// the same posture postgres/mysql take — recovery is `redis:start`.
func LoadState() (State, error) {
	all, err := state.Load()
	if err != nil {
		return State{}, err
	}
	raw, ok := all[stateKey]
	if !ok {
		return State{}, nil
	}
	var s State
	if err := json.Unmarshal(raw, &s); err != nil {
		fmt.Fprintf(os.Stderr, "redis: state slice corrupt (%v); treating as empty\n", err)
		return State{}, nil
	}
	return s, nil
}

// SaveState writes the redis slice, preserving other services' slices.
func SaveState(s State) error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	payload, err := json.Marshal(s)
	if err != nil {
		return err
	}
	all[stateKey] = payload
	return state.Save(all)
}

// SetWanted updates the wanted-state and persists. Rejects values
// outside the WantedRunning/WantedStopped set so a typo can't silently
// persist garbage that IsWanted will later read as "not running" (and
// stop the process).
func SetWanted(wanted string) error {
	if wanted != WantedRunning && wanted != WantedStopped {
		return fmt.Errorf("redis: invalid wanted state %q (want %q or %q)", wanted, WantedRunning, WantedStopped)
	}
	return SaveState(State{Wanted: wanted})
}

// RemoveState drops the redis entry from state.json entirely. Used by
// `redis:uninstall` so a fresh install doesn't inherit a stale wanted
// flag from before.
func RemoveState() error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	delete(all, stateKey)
	return state.Save(all)
}
