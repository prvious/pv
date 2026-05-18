package control

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestFileStorePersistsDesiredAndObservedSeparately(t *testing.T) {
	ctx := context.Background()
	store := NewFileStore(filepath.Join(t.TempDir(), "pv.json"))

	desired := DesiredResource{
		Resource: ResourceMago,
		Version:  "1.2.3",
	}
	if err := store.PutDesired(ctx, desired); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}

	reloaded := NewFileStore(store.Path())
	gotDesired, ok, err := reloaded.Desired(ctx, ResourceMago)
	if err != nil {
		t.Fatalf("Desired returned error: %v", err)
	}
	if !ok {
		t.Fatal("Desired did not find mago resource")
	}
	if gotDesired != desired {
		t.Fatalf("Desired = %#v, want %#v", gotDesired, desired)
	}

	if _, ok, err := reloaded.Observed(ctx, ResourceMago); err != nil {
		t.Fatalf("Observed returned error: %v", err)
	} else if ok {
		t.Fatal("Observed found status before controller wrote one")
	}

	observed := ObservedStatus{
		Resource:          ResourceMago,
		DesiredVersion:    "1.2.3",
		State:             StateReady,
		LastReconcileTime: "2026-05-15T17:30:00Z",
	}
	if err := reloaded.PutObserved(ctx, observed); err != nil {
		t.Fatalf("PutObserved returned error: %v", err)
	}

	again := NewFileStore(store.Path())
	gotObserved, ok, err := again.Observed(ctx, ResourceMago)
	if err != nil {
		t.Fatalf("Observed after write returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find mago status")
	}
	if gotObserved != observed {
		t.Fatalf("Observed = %#v, want %#v", gotObserved, observed)
	}

	gotDesired, ok, err = again.Desired(ctx, ResourceMago)
	if err != nil {
		t.Fatalf("Desired after observed write returned error: %v", err)
	}
	if !ok {
		t.Fatal("Desired missing after observed write")
	}
	if gotDesired != desired {
		t.Fatalf("Desired after observed write = %#v, want %#v", gotDesired, desired)
	}
}

func TestValidateVersionRejectsUnsafeVersion(t *testing.T) {
	for _, version := range []string{"", "../mago", "1.0/evil", "two words"} {
		if err := ValidateVersion(version); err == nil {
			t.Fatalf("ValidateVersion(%q) returned nil, want error", version)
		}
	}
}

func TestFileStoreRecordsSchemaVersionAndAppliedMigrations(t *testing.T) {
	ctx := context.Background()
	store := NewFileStore(filepath.Join(t.TempDir(), "pv.db"))

	if err := store.Migrate(ctx); err != nil {
		t.Fatalf("Migrate returned error: %v", err)
	}
	version, err := store.SchemaVersion(ctx)
	if err != nil {
		t.Fatalf("SchemaVersion returned error: %v", err)
	}
	if version != CurrentSchemaVersion {
		t.Fatalf("schema version = %d, want %d", version, CurrentSchemaVersion)
	}

	data, err := os.ReadFile(store.Path())
	if err != nil {
		t.Fatalf("ReadFile returned error: %v", err)
	}
	if !strings.Contains(string(data), `"id": "0001_initial_json_store"`) {
		t.Fatalf("store file did not record initial migration:\n%s", data)
	}
}

func TestFileStoreRejectsCorruptedState(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "pv.db")
	if err := os.WriteFile(path, []byte("{not json"), 0o600); err != nil {
		t.Fatalf("WriteFile returned error: %v", err)
	}

	_, _, err := NewFileStore(path).Desired(ctx, ResourceMago)
	if err == nil {
		t.Fatal("Desired returned nil error")
	}
	if !strings.Contains(err.Error(), "load store state") {
		t.Fatalf("error = %v, want clear load store state message", err)
	}
}

func TestFileStoreRejectsNewerSchema(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "pv.db")
	if err := os.WriteFile(path, []byte(`{"schema_version":99,"desired":{},"observed":{}}`), 0o600); err != nil {
		t.Fatalf("WriteFile returned error: %v", err)
	}

	_, err := NewFileStore(path).SchemaVersion(ctx)
	if err == nil {
		t.Fatal("SchemaVersion returned nil error")
	}
	if !strings.Contains(err.Error(), "newer than supported") {
		t.Fatalf("error = %v, want newer schema message", err)
	}
}
