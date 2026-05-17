package cli

import (
	"bytes"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/prvious/pv/internal/control"
	"github.com/prvious/pv/internal/resources/mago"
)

func TestRunHelpWritesRewriteHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"help"}, &stdout, &stderr)

	if err != nil {
		t.Fatalf("Run help returned error: %v", err)
	}
	if stderr.Len() != 0 {
		t.Fatalf("Run help wrote stderr: %q", stderr.String())
	}

	out := stdout.String()
	for _, want := range []string{
		"pv rewrite control plane",
		"Usage:",
		"pv <command>",
		"composer:install",
		"mago:install",
		"php:install",
		"version",
	} {
		if !strings.Contains(out, want) {
			t.Fatalf("help output missing %q:\n%s", want, out)
		}
	}
}

func TestRunVersionWritesPipeableVersion(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"version"}, &stdout, &stderr)

	if err != nil {
		t.Fatalf("Run version returned error: %v", err)
	}
	if stderr.Len() != 0 {
		t.Fatalf("Run version wrote stderr: %q", stderr.String())
	}
	if got := stdout.String(); got != "pv dev\n" {
		t.Fatalf("version output = %q, want %q", got, "pv dev\n")
	}
}

func TestRunUnknownCommandReturnsUsageError(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"link"}, &stdout, &stderr)

	if !errors.Is(err, ErrUsage) {
		t.Fatalf("Run unknown command error = %v, want ErrUsage", err)
	}
	if stdout.Len() != 0 {
		t.Fatalf("Run unknown command wrote stdout: %q", stdout.String())
	}
	if got := stderr.String(); !strings.Contains(got, `unknown command "link"`) {
		t.Fatalf("stderr = %q, want unknown command message", got)
	}
}

func TestRunMagoInstallRecordsDesiredStateWithoutInstalling(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"mago:install", "1.2.3"}, &stdout, &stderr)

	if err != nil {
		t.Fatalf("Run mago install returned error: %v", err)
	}
	if stdout.Len() != 0 {
		t.Fatalf("Run mago install wrote stdout: %q", stdout.String())
	}
	if got := stderr.String(); !strings.Contains(got, "requested mago 1.2.3") {
		t.Fatalf("stderr = %q, want requested message", got)
	}

	store := control.NewFileStore(filepath.Join(home, ".pv", "state", "pv.db"))
	desired, ok, err := store.Desired(t.Context(), control.ResourceMago)
	if err != nil {
		t.Fatalf("Desired returned error: %v", err)
	}
	if !ok {
		t.Fatal("Desired did not find mago resource")
	}
	if desired.Version != "1.2.3" {
		t.Fatalf("desired version = %q, want 1.2.3", desired.Version)
	}
	if _, ok, err := store.Observed(t.Context(), control.ResourceMago); err != nil {
		t.Fatalf("Observed returned error: %v", err)
	} else if ok {
		t.Fatal("mago install command wrote observed status directly")
	}

	installer := mago.NewMarkerInstaller(filepath.Join(home, ".pv"))
	if installer.Installed("1.2.3") {
		t.Fatal("mago install command installed marker directly")
	}
}

func TestRunStatusReportsDesiredAndObservedStatus(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	store := control.NewFileStore(filepath.Join(home, ".pv", "state", "pv.db"))
	ctx := t.Context()
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource: control.ResourceMago,
		Version:  "1.2.3",
	}); err != nil {
		t.Fatalf("PutDesired returned error: %v", err)
	}
	if err := store.PutObserved(ctx, control.ObservedStatus{
		Resource:          control.ResourceMago,
		DesiredVersion:    "1.2.3",
		State:             control.StateReady,
		LastReconcileTime: "2026-05-15T18:00:00Z",
	}); err != nil {
		t.Fatalf("PutObserved returned error: %v", err)
	}
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"status"}, &stdout, &stderr)

	if err != nil {
		t.Fatalf("Run status returned error: %v", err)
	}
	if stdout.Len() != 0 {
		t.Fatalf("Run status wrote stdout: %q", stdout.String())
	}
	for _, want := range []string{
		"desired: mago 1.2.3 install",
		"observed: mago 1.2.3 ready",
		"last reconcile: 2026-05-15T18:00:00Z",
	} {
		if !strings.Contains(stderr.String(), want) {
			t.Fatalf("status output missing %q:\n%s", want, stderr.String())
		}
	}
}

func TestControlPlaneTracerRecordsDesiredThenObserved(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	ctx := t.Context()
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"mago:install", "1.2.3"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run mago install returned error: %v", err)
	}

	store := control.NewFileStore(filepath.Join(home, ".pv", "state", "pv.db"))
	controller := mago.Controller{
		Store:     store,
		Installer: mago.NewMarkerInstaller(filepath.Join(home, ".pv")),
		Clock:     fixedClock("2026-05-15T18:00:00Z"),
	}
	if err := controller.Reconcile(ctx); err != nil {
		t.Fatalf("Reconcile returned error: %v", err)
	}

	desired, ok, err := store.Desired(ctx, control.ResourceMago)
	if err != nil {
		t.Fatalf("Desired returned error: %v", err)
	}
	if !ok {
		t.Fatal("Desired missing after reconcile")
	}
	if desired.Version != "1.2.3" {
		t.Fatalf("desired version = %q, want 1.2.3", desired.Version)
	}
	observed, ok, err := store.Observed(ctx, control.ResourceMago)
	if err != nil {
		t.Fatalf("Observed returned error: %v", err)
	}
	if !ok {
		t.Fatal("Observed missing after reconcile")
	}
	if observed.State != control.StateReady {
		t.Fatalf("observed state = %q, want ready", observed.State)
	}
}

func TestRunPHPInstallRecordsDesiredRuntimeState(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"php:install", "8.4"}, &stdout, &stderr)

	if err != nil {
		t.Fatalf("Run php install returned error: %v", err)
	}
	if stdout.Len() != 0 {
		t.Fatalf("Run php install wrote stdout: %q", stdout.String())
	}
	if got := stderr.String(); !strings.Contains(got, "requested php 8.4 install") {
		t.Fatalf("stderr = %q, want requested message", got)
	}

	store := control.NewFileStore(filepath.Join(home, ".pv", "state", "pv.db"))
	desired, ok, err := store.Desired(t.Context(), control.ResourcePHP)
	if err != nil {
		t.Fatalf("Desired returned error: %v", err)
	}
	if !ok {
		t.Fatal("Desired did not find PHP resource")
	}
	if desired.Version != "8.4" {
		t.Fatalf("desired version = %q, want 8.4", desired.Version)
	}
}

func TestRunComposerInstallRecordsRuntimeDependency(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"composer:install", "2.8.0", "--php", "8.4"}, &stdout, &stderr)

	if err != nil {
		t.Fatalf("Run composer install returned error: %v", err)
	}
	if stdout.Len() != 0 {
		t.Fatalf("Run composer install wrote stdout: %q", stdout.String())
	}
	if got := stderr.String(); !strings.Contains(got, "requested composer 2.8.0 install with php 8.4") {
		t.Fatalf("stderr = %q, want requested message", got)
	}

	store := control.NewFileStore(filepath.Join(home, ".pv", "state", "pv.db"))
	desired, ok, err := store.Desired(t.Context(), control.ResourceComposer)
	if err != nil {
		t.Fatalf("Desired returned error: %v", err)
	}
	if !ok {
		t.Fatal("Desired did not find Composer resource")
	}
	if desired.Version != "2.8.0" {
		t.Fatalf("desired version = %q, want 2.8.0", desired.Version)
	}
	if desired.RuntimeVersion != "8.4" {
		t.Fatalf("runtime version = %q, want 8.4", desired.RuntimeVersion)
	}
}

func TestRunStatusReportsPHPAndComposer(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	ctx := t.Context()
	store := control.NewFileStore(filepath.Join(home, ".pv", "state", "pv.db"))
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource: control.ResourcePHP,
		Version:  "8.4",
	}); err != nil {
		t.Fatalf("PutDesired PHP returned error: %v", err)
	}
	if err := store.PutObserved(ctx, control.ObservedStatus{
		Resource:          control.ResourcePHP,
		DesiredVersion:    "8.4",
		State:             control.StateReady,
		LastReconcileTime: "2026-05-15T19:00:00Z",
	}); err != nil {
		t.Fatalf("PutObserved PHP returned error: %v", err)
	}
	if err := store.PutDesired(ctx, control.DesiredResource{
		Resource:       control.ResourceComposer,
		Version:        "2.8.0",
		RuntimeVersion: "8.4",
	}); err != nil {
		t.Fatalf("PutDesired Composer returned error: %v", err)
	}
	if err := store.PutObserved(ctx, control.ObservedStatus{
		Resource:          control.ResourceComposer,
		DesiredVersion:    "2.8.0",
		RuntimeVersion:    "8.4",
		State:             control.StateBlocked,
		LastReconcileTime: "2026-05-15T19:10:00Z",
		LastError:         "PHP runtime 8.4 is not installed",
		NextAction:        "run pv php:install 8.4",
	}); err != nil {
		t.Fatalf("PutObserved Composer returned error: %v", err)
	}
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"status"}, &stdout, &stderr)

	if err != nil {
		t.Fatalf("Run status returned error: %v", err)
	}
	if stdout.Len() != 0 {
		t.Fatalf("Run status wrote stdout: %q", stdout.String())
	}
	for _, want := range []string{
		"desired: php 8.4 install",
		"observed: php 8.4 ready",
		"desired: composer 2.8.0 install with php 8.4",
		"observed: composer 2.8.0 blocked",
		"last error: PHP runtime 8.4 is not installed",
		"next action: run pv php:install 8.4",
	} {
		if !strings.Contains(stderr.String(), want) {
			t.Fatalf("status output missing %q:\n%s", want, stderr.String())
		}
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
