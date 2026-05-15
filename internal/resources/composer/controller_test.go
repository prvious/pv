package composer

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/resources/php"
)

func TestControllerBlocksComposerWhenRuntimeIsMissing(t *testing.T) {
	ctx := t.Context()
	root := t.TempDir()
	store := control.NewFileStore(filepath.Join(root, "pv.json"))
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource:       control.ResourceComposer,
		Version:        "2.8.0",
		RuntimeVersion: "8.4",
	}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}

	installer := NewMarkerInstaller(root)
	controller := Controller{
		Store:     store,
		Installer: installer,
		Runtime:   php.NewMarkerInstaller(root),
		Clock:     fixedClock("2026-05-15T19:10:00Z"),
	}

	if err := controller.Reconcile(ctx); err != nil {
		t.Fatalf("Reconcile returned error: %v", err)
	}

	if installer.Installed("2.8.0") {
		t.Fatal("Composer installed without its PHP runtime")
	}
	if installer.ShimExists() {
		t.Fatal("Composer shim exists without its PHP runtime")
	}

	observed, ok, err := store.Observed(ctx, control.ResourceComposer)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find Composer status")
	}
	if observed.State != control.StateBlocked {
		t.Fatalf("State = %q, want blocked", observed.State)
	}
	if observed.LastError != "PHP runtime 8.4 is not installed" {
		t.Fatalf("LastError = %q", observed.LastError)
	}
	if !strings.Contains(observed.NextAction, "php:install 8.4") {
		t.Fatalf("NextAction = %q, want php:install guidance", observed.NextAction)
	}
}

func TestControllerInstallsComposerAndExposesShimWhenRuntimeExists(t *testing.T) {
	ctx := t.Context()
	root := t.TempDir()
	store := control.NewFileStore(filepath.Join(root, "pv.json"))
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource:       control.ResourceComposer,
		Version:        "2.8.0",
		RuntimeVersion: "8.4",
	}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}
	runtime := php.NewMarkerInstaller(root)
	if err := runtime.Install(ctx, "8.4"); err != nil {
		t.Fatalf("PHP Install returned error: %v", err)
	}

	installer := NewMarkerInstaller(root)
	controller := Controller{
		Store:     store,
		Installer: installer,
		Runtime:   runtime,
		Clock:     fixedClock("2026-05-15T19:10:00Z"),
	}

	if err := controller.Reconcile(ctx); err != nil {
		t.Fatalf("Reconcile returned error: %v", err)
	}

	if !installer.Installed("2.8.0") {
		t.Fatal("Composer marker was not installed")
	}
	shim, err := os.ReadFile(filepath.Join(root, "bin", "composer"))
	if err != nil {
		t.Fatalf("ReadFile shim returned error: %v", err)
	}
	for _, want := range []string{"composer 2.8.0", "php 8.4"} {
		if !strings.Contains(string(shim), want) {
			t.Fatalf("shim missing %q:\n%s", want, string(shim))
		}
	}

	observed, ok, err := store.Observed(ctx, control.ResourceComposer)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed did not find Composer status")
	}
	want := control.ObservedStatus{
		Resource:          control.ResourceComposer,
		DesiredVersion:    "2.8.0",
		RuntimeVersion:    "8.4",
		State:             control.StateReady,
		LastReconcileTime: "2026-05-15T19:10:00Z",
	}
	if observed != want {
		t.Fatalf("Observed = %#v, want %#v", observed, want)
	}
}

func TestMarkerInstallerReplacesComposerShimAtomically(t *testing.T) {
	ctx := t.Context()
	root := t.TempDir()
	bin := filepath.Join(root, "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatalf("MkdirAll returned error: %v", err)
	}
	shim := filepath.Join(bin, "composer")
	if err := os.WriteFile(shim, []byte("old shim\n"), 0o755); err != nil {
		t.Fatalf("WriteFile old shim returned error: %v", err)
	}

	installer := NewMarkerInstaller(root)
	if err := installer.Install(ctx, InstallRequest{
		Version:        "2.8.0",
		RuntimeVersion: "8.4",
	}); err != nil {
		t.Fatalf("Install returned error: %v", err)
	}

	got, err := os.ReadFile(shim)
	if err != nil {
		t.Fatalf("ReadFile shim returned error: %v", err)
	}
	if string(got) == "old shim\n" {
		t.Fatal("composer shim was not replaced")
	}
	matches, err := filepath.Glob(filepath.Join(bin, ".composer-*"))
	if err != nil {
		t.Fatalf("Glob returned error: %v", err)
	}
	if len(matches) != 0 {
		t.Fatalf("temporary shim files were not cleaned up: %v", matches)
	}
}

func fixedClock(value string) func() time.Time {
	parsed, err := time.Parse(time.RFC3339, value)
	if err != nil {
		panic(err)
	}
	return func() time.Time {
		return parsed
	}
}
