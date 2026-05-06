package postgres

import (
	"encoding/json"

	"github.com/prvious/pv/internal/state"
)

const stateKey = "postgres"

// MajorState is the per-major sub-record of postgres state.
type MajorState struct {
	Wanted string `json:"wanted"`
}

// State is the postgres slice of ~/.pv/data/state.json.
type State struct {
	Majors map[string]MajorState `json:"majors"`
}

// LoadState reads the postgres slice. Missing or empty → zero-value state.
func LoadState() (State, error) {
	all, err := state.Load()
	if err != nil {
		return State{Majors: map[string]MajorState{}}, err
	}
	raw, ok := all[stateKey]
	if !ok {
		return State{Majors: map[string]MajorState{}}, nil
	}
	var s State
	if err := json.Unmarshal(raw, &s); err != nil {
		return State{Majors: map[string]MajorState{}}, nil
	}
	if s.Majors == nil {
		s.Majors = map[string]MajorState{}
	}
	return s, nil
}

// SaveState writes the postgres slice, preserving other services' slices.
func SaveState(s State) error {
	all, err := state.Load()
	if err != nil {
		return err
	}
	if s.Majors == nil {
		s.Majors = map[string]MajorState{}
	}
	payload, err := json.Marshal(s)
	if err != nil {
		return err
	}
	all[stateKey] = payload
	return state.Save(all)
}

// SetWanted updates the wanted-state for one major and persists.
func SetWanted(major, wanted string) error {
	s, err := LoadState()
	if err != nil {
		return err
	}
	s.Majors[major] = MajorState{Wanted: wanted}
	return SaveState(s)
}

// RemoveMajor drops a major's entry from state and persists.
func RemoveMajor(major string) error {
	s, err := LoadState()
	if err != nil {
		return err
	}
	delete(s.Majors, major)
	return SaveState(s)
}
