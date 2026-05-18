package rustfs

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/prvious/pv/internal/state"
)

const stateKey = "rustfs"

const (
	WantedRunning = "running"
	WantedStopped = "stopped"
)

type VersionState struct {
	Wanted string `json:"wanted"`
}

type State struct {
	Versions map[string]VersionState `json:"versions"`
}

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
		fmt.Fprintf(os.Stderr, "rustfs: state slice corrupt (%v); treating as empty\n", err)
		return State{Versions: map[string]VersionState{}}, nil
	}
	if s.Versions == nil {
		s.Versions = map[string]VersionState{}
	}
	return s, nil
}

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

func SetWanted(version, wanted string) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	if wanted != WantedRunning && wanted != WantedStopped {
		return fmt.Errorf("rustfs: invalid wanted state %q (want %q or %q)", wanted, WantedRunning, WantedStopped)
	}
	s, err := LoadState()
	if err != nil {
		return err
	}
	s.Versions[version] = VersionState{Wanted: wanted}
	return SaveState(s)
}

func RemoveVersion(version string) error {
	if err := ValidateVersion(version); err != nil {
		return err
	}
	s, err := LoadState()
	if err != nil {
		return err
	}
	delete(s.Versions, version)
	return SaveState(s)
}
