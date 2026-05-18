package state

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestLoad_MissingFile_ReturnsEmpty(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s, err := Load()
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if got := len(s); got != 0 {
		t.Errorf("expected empty state, got %d entries", got)
	}
}

func TestSaveLoad_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	s := State{}
	s["postgres"] = json.RawMessage(`{"majors":{"17":{"wanted":"running"}}}`)
	if err := Save(s); err != nil {
		t.Fatalf("Save: %v", err)
	}
	got, err := Load()
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	raw, ok := got["postgres"]
	if !ok {
		t.Fatal("expected postgres key after round-trip")
	}
	// Verify the JSON unmarshals correctly
	var data map[string]map[string]map[string]string
	if err := json.Unmarshal(raw, &data); err != nil {
		t.Errorf("failed to unmarshal: %v", err)
	}
	if data["majors"]["17"]["wanted"] != "running" {
		t.Errorf("unexpected payload: %v", data)
	}
}

func TestLoad_CorruptFile_ReturnsEmptyWithWarning(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("HOME", tmp)
	dataDir := filepath.Join(tmp, ".pv", "data")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "state.json"), []byte("not json"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	s, err := Load()
	if err != nil {
		t.Fatalf("Load should tolerate corruption, got: %v", err)
	}
	if len(s) != 0 {
		t.Errorf("expected empty state on corruption, got %d", len(s))
	}
}
