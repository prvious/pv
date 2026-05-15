package control

import (
	"context"
	"path/filepath"
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
