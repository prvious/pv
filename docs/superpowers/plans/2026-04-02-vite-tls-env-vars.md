# Vite TLS Env Vars Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Valet cert directory with pv-native cert storage at `~/.pv/data/certs/` and use `VITE_DEV_SERVER_*` env vars instead of Valet auto-detection.

**Architecture:** Rewrite `internal/certs/valet.go` to use pv-owned cert storage, add `SetViteTLSStep` automation step, wire up the gate in settings/pipeline, remove all Valet config code from callers.

**Tech Stack:** Go, x509 certificates, cobra CLI

**Spec:** `docs/superpowers/specs/2026-04-02-vite-tls-env-vars-design.md`

---

### Task 1: Rewrite cert storage to use `~/.pv/data/certs/`

**Files:**
- Modify: `internal/certs/valet.go`
- Modify: `internal/certs/valet_test.go`

- [ ] **Step 1: Write tests for new cert storage functions**

Replace the entire contents of `internal/certs/valet_test.go` with:

```go
package certs

import (
	"os"
	"path/filepath"
	"testing"
)

func TestCertsDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	dir := CertsDir()
	expected := filepath.Join(home, ".pv", "data", "certs")
	if dir != expected {
		t.Errorf("CertsDir() = %q, want %q", dir, expected)
	}
}

func TestCertPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	p := CertPath("myapp.test")
	expected := filepath.Join(home, ".pv", "data", "certs", "myapp.test.crt")
	if p != expected {
		t.Errorf("CertPath() = %q, want %q", p, expected)
	}
}

func TestKeyPath(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	p := KeyPath("myapp.test")
	expected := filepath.Join(home, ".pv", "data", "certs", "myapp.test.key")
	if p != expected {
		t.Errorf("KeyPath() = %q, want %q", p, expected)
	}
}

func TestGenerateSiteTLS(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	// GenerateSiteTLS requires Caddy CA — test that it returns an error
	// when no CA exists (same behavior as before).
	err := GenerateSiteTLS("myapp.test")
	if err == nil {
		t.Fatal("expected error when CA doesn't exist")
	}
}

func TestRemoveSiteTLS(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	certsDir := CertsDir()
	os.MkdirAll(certsDir, 0755)

	certPath := filepath.Join(certsDir, "myapp.test.crt")
	keyPath := filepath.Join(certsDir, "myapp.test.key")
	os.WriteFile(certPath, []byte("cert"), 0644)
	os.WriteFile(keyPath, []byte("key"), 0600)

	if err := RemoveSiteTLS("myapp.test"); err != nil {
		t.Fatalf("RemoveSiteTLS() error = %v", err)
	}

	if _, err := os.Stat(certPath); !os.IsNotExist(err) {
		t.Error("cert file should be removed")
	}
	if _, err := os.Stat(keyPath); !os.IsNotExist(err) {
		t.Error("key file should be removed")
	}
}

func TestRemoveSiteTLS_NonExistent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	if err := RemoveSiteTLS("nonexistent.test"); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestRemoveLinkedCerts(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	certsDir := CertsDir()
	os.MkdirAll(certsDir, 0755)

	for _, name := range []string{"app1.test", "app2.test", "other.test"} {
		os.WriteFile(filepath.Join(certsDir, name+".crt"), []byte("cert"), 0644)
		os.WriteFile(filepath.Join(certsDir, name+".key"), []byte("key"), 0600)
	}

	if err := RemoveLinkedCerts([]string{"app1.test", "app2.test"}); err != nil {
		t.Fatalf("RemoveLinkedCerts() error = %v", err)
	}

	for _, name := range []string{"app1.test", "app2.test"} {
		if _, err := os.Stat(filepath.Join(certsDir, name+".crt")); !os.IsNotExist(err) {
			t.Errorf("%s.crt should be removed", name)
		}
	}

	if _, err := os.Stat(filepath.Join(certsDir, "other.test.crt")); err != nil {
		t.Error("other.test.crt should NOT be removed")
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test ./internal/certs/ -run "TestCertsDir|TestCertPath|TestKeyPath|TestRemoveSiteTLS$|TestRemoveLinkedCerts$" -v`
Expected: FAIL — functions don't exist yet.

- [ ] **Step 3: Rewrite `internal/certs/valet.go`**

Replace the entire contents of `internal/certs/valet.go` with:

```go
package certs

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// CertsDir returns ~/.pv/data/certs/.
func CertsDir() string {
	return filepath.Join(config.DataDir(), "certs")
}

// CertPath returns the full path to a site certificate.
func CertPath(hostname string) string {
	return filepath.Join(CertsDir(), hostname+".crt")
}

// KeyPath returns the full path to a site private key.
func KeyPath(hostname string) string {
	return filepath.Join(CertsDir(), hostname+".key")
}

// GenerateSiteTLS generates a TLS cert/key pair for hostname and places them
// in the pv certs directory. Uses Caddy's local CA to sign the certificate.
func GenerateSiteTLS(hostname string) error {
	caCertPath := config.CACertPath()
	caKeyPath := config.CAKeyPath()

	if _, err := os.Stat(caCertPath); err != nil {
		return fmt.Errorf("Caddy CA not found at %s (run pv start first to generate it)", caCertPath)
	}
	if _, err := os.Stat(caKeyPath); err != nil {
		return fmt.Errorf("Caddy CA key not found at %s", caKeyPath)
	}

	certsDir := CertsDir()
	if err := os.MkdirAll(certsDir, 0755); err != nil {
		return fmt.Errorf("cannot create certs dir: %w", err)
	}

	certPath := CertPath(hostname)
	keyPath := KeyPath(hostname)

	return GenerateSiteCert(hostname, caCertPath, caKeyPath, certPath, keyPath)
}

// RemoveSiteTLS removes the TLS cert/key pair for hostname.
// Returns nil if the files do not exist.
func RemoveSiteTLS(hostname string) error {
	var errs []error
	for _, ext := range []string{".crt", ".key"} {
		if err := os.Remove(filepath.Join(CertsDir(), hostname+ext)); err != nil && !os.IsNotExist(err) {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

// RemoveLinkedCerts removes TLS cert/key pairs for the given hostnames.
func RemoveLinkedCerts(hostnames []string) error {
	var errs []error
	for _, h := range hostnames {
		for _, ext := range []string{".crt", ".key"} {
			if err := os.Remove(filepath.Join(CertsDir(), h+ext)); err != nil && !os.IsNotExist(err) {
				errs = append(errs, err)
			}
		}
	}
	return errors.Join(errs...)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test ./internal/certs/ -v`
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/certs/valet.go internal/certs/valet_test.go
git commit -m "Move cert storage from ~/.config/valet/ to ~/.pv/data/certs/

Replace Valet-specific cert functions with pv-native storage.
Remove EnsureValetConfig, ValetConfigDir, ValetCertsDir,
RemoveConfig, and all config.json management."
```

### Task 2: Update callers that reference removed Valet functions

**Files:**
- Modify: `internal/automation/steps/generate_tls_cert.go`
- Modify: `cmd/setup.go`
- Modify: `cmd/uninstall.go`

- [ ] **Step 1: Update `generate_tls_cert.go` — remove `EnsureValetConfig` call**

Replace the entire `Run` method in `internal/automation/steps/generate_tls_cert.go` with:

```go
func (s *GenerateTLSCertStep) Run(ctx *automation.Context) (string, error) {
	hostname := fmt.Sprintf("%s.%s", ctx.ProjectName, ctx.TLD)
	if err := certs.GenerateSiteTLS(hostname); err != nil {
		return "", fmt.Errorf("TLS cert not generated for %s: %w", hostname, err)
	}
	return hostname, nil
}
```

- [ ] **Step 2: Update `cmd/setup.go` — remove `EnsureValetConfig` call**

In `cmd/setup.go`, remove these lines (around lines 149-152):

```go
			// Write Valet-compatible config for Vite TLS auto-detection.
			if err := certs.EnsureValetConfig(tld); err != nil {
				ui.Subtle(fmt.Sprintf("Vite TLS config: %v", err))
			}
```

Also remove the `certs` import from `cmd/setup.go` if it becomes unused (check if any other certs function is still called in the file).

- [ ] **Step 3: Update `cmd/uninstall.go` — remove `RemoveConfig` call**

In `cmd/uninstall.go`, remove these lines (around lines 239-241):

```go
		if err := certs.RemoveConfig(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not remove Valet config: %v", err))
		}
```

Keep the `certs.RemoveLinkedCerts(hostnames)` call — it still works (now removes from `~/.pv/data/certs/`).

- [ ] **Step 4: Verify build and tests**

Run: `go build ./... && go test ./... && go vet ./...`
Expected: All clean.

- [ ] **Step 5: Commit**

```bash
git add internal/automation/steps/generate_tls_cert.go cmd/setup.go cmd/uninstall.go
git commit -m "Remove Valet config calls from callers

EnsureValetConfig and RemoveConfig are gone. GenerateTLSCertStep
now calls GenerateSiteTLS directly. RemoveLinkedCerts still used
in uninstall (points to new cert dir)."
```

### Task 3: Add `SetViteTLSStep` automation step

**Files:**
- Modify: `internal/laravel/steps.go`
- Modify: `internal/laravel/steps_test.go`

- [ ] **Step 1: Write tests for `SetViteTLSStep`**

Add these tests to `internal/laravel/steps_test.go` (after the existing `SetAppURLStep` tests):

```go
// --- SetViteTLSStep tests ---

func TestSetViteTLSStep_ShouldRun_TrueForLaravel(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_URL=http://localhost"), 0644)

	step := &SetViteTLSStep{}
	ctx := &automation.Context{ProjectType: "laravel", ProjectPath: dir}
	if !step.ShouldRun(ctx) {
		t.Error("expected ShouldRun=true for laravel with .env")
	}
}

func TestSetViteTLSStep_ShouldRun_FalseWhenNoEnv(t *testing.T) {
	dir := t.TempDir()
	step := &SetViteTLSStep{}
	ctx := &automation.Context{ProjectType: "laravel", ProjectPath: dir}
	if step.ShouldRun(ctx) {
		t.Error("expected ShouldRun=false when no .env")
	}
}

func TestSetViteTLSStep_ShouldRun_FalseForPHP(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte(""), 0644)
	step := &SetViteTLSStep{}
	ctx := &automation.Context{ProjectType: "php", ProjectPath: dir}
	if step.ShouldRun(ctx) {
		t.Error("expected ShouldRun=false for php")
	}
}

func TestSetViteTLSStep_Run(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, ".env"), []byte("APP_URL=https://myapp.test\n"), 0644)

	step := &SetViteTLSStep{}
	ctx := &automation.Context{
		ProjectPath: dir,
		ProjectName: "myapp",
		TLD:         "test",
	}

	result, err := step.Run(ctx)
	if err != nil {
		t.Fatalf("Run() error = %v", err)
	}
	if result == "" {
		t.Error("expected non-empty result")
	}

	env, err := services.ReadDotEnv(filepath.Join(dir, ".env"))
	if err != nil {
		t.Fatalf("ReadDotEnv: %v", err)
	}

	certPath := certs.CertPath("myapp.test")
	keyPath := certs.KeyPath("myapp.test")

	if env["VITE_DEV_SERVER_CERT"] != certPath {
		t.Errorf("VITE_DEV_SERVER_CERT = %q, want %q", env["VITE_DEV_SERVER_CERT"], certPath)
	}
	if env["VITE_DEV_SERVER_KEY"] != keyPath {
		t.Errorf("VITE_DEV_SERVER_KEY = %q, want %q", env["VITE_DEV_SERVER_KEY"], keyPath)
	}
}
```

Note: You will need to add `"github.com/prvious/pv/internal/certs"` to the test file imports. Check existing imports first — `services` should already be imported.

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test ./internal/laravel/ -run "TestSetViteTLSStep" -v`
Expected: FAIL — `SetViteTLSStep` not defined.

- [ ] **Step 3: Implement `SetViteTLSStep`**

Add this to `internal/laravel/steps.go`, after `SetAppURLStep` (after line 111):

```go
// --- SetViteTLSStep ---

// SetViteTLSStep sets VITE_DEV_SERVER_KEY and VITE_DEV_SERVER_CERT in .env
// so laravel-vite-plugin can find the TLS certificate for the dev server.
type SetViteTLSStep struct{}

var _ automation.Step = (*SetViteTLSStep)(nil)

func (s *SetViteTLSStep) Label() string  { return "Set Vite TLS" }
func (s *SetViteTLSStep) Gate() string   { return "set_vite_tls" }
func (s *SetViteTLSStep) Critical() bool { return false }

func (s *SetViteTLSStep) ShouldRun(ctx *automation.Context) bool {
	return isLaravel(ctx.ProjectType) && HasEnvFile(ctx.ProjectPath)
}

func (s *SetViteTLSStep) Run(ctx *automation.Context) (string, error) {
	tld := ctx.TLD
	if tld == "" {
		tld = "test"
	}
	hostname := fmt.Sprintf("%s.%s", ctx.ProjectName, tld)
	envPath := filepath.Join(ctx.ProjectPath, ".env")
	vars := map[string]string{
		"VITE_DEV_SERVER_CERT": certs.CertPath(hostname),
		"VITE_DEV_SERVER_KEY":  certs.KeyPath(hostname),
	}
	if err := services.MergeDotEnv(envPath, "", vars); err != nil {
		return "", fmt.Errorf("set Vite TLS: %w", err)
	}
	return hostname, nil
}
```

You will need to add `"github.com/prvious/pv/internal/certs"` to the imports in `steps.go`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test ./internal/laravel/ -v`
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/laravel/steps.go internal/laravel/steps_test.go
git commit -m "Add SetViteTLSStep automation step

Writes VITE_DEV_SERVER_KEY and VITE_DEV_SERVER_CERT to .env
pointing to pv's cert storage. Runs for Laravel projects with
.env files."
```

### Task 4: Wire up settings gate and pipeline

**Files:**
- Modify: `internal/config/settings.go`
- Modify: `internal/automation/pipeline.go`
- Modify: `cmd/link.go`

- [ ] **Step 1: Add `SetViteTLS` field to `Automation` struct**

In `internal/config/settings.go`, add after the `SetAppURL` field (line 60):

```go
	SetViteTLS        AutoMode `yaml:"set_vite_tls,omitempty"`
```

- [ ] **Step 2: Add default value**

In the `DefaultAutomation()` function, add after `SetAppURL: AutoOn,` (line 88):

```go
		SetViteTLS:         AutoOn,
```

- [ ] **Step 3: Add validation in `applyAutomationDefaults`**

In `applyAutomationDefaults`, add after the `SetAppURL` validation block (after line 123):

```go
	if !validAutoMode(a.SetViteTLS) {
		a.SetViteTLS = d.SetViteTLS
	}
```

- [ ] **Step 4: Add gate case in pipeline**

In `internal/automation/pipeline.go`, add a case in the `automationMode` switch after the `set_app_url` case (after line 124):

```go
	case "set_vite_tls":
		return a.SetViteTLS
```

- [ ] **Step 5: Add step to pipeline in `cmd/link.go`**

In `cmd/link.go`, add `&laravel.SetViteTLSStep{}` after `&laravel.SetAppURLStep{}` in the `allSteps` slice:

```go
			&laravel.SetAppURLStep{},
			&laravel.SetViteTLSStep{},
```

- [ ] **Step 6: Verify build and full test suite**

Run: `go build ./... && go test ./... && gofmt -l . && go vet ./...`
Expected: All clean.

- [ ] **Step 7: Commit**

```bash
git add internal/config/settings.go internal/automation/pipeline.go cmd/link.go
git commit -m "Wire SetViteTLS gate into settings, pipeline, and link command

New automation gate 'set_vite_tls' defaults to on. Step runs
after SetAppURLStep in the link pipeline."
```

### Task 5: Update settings tests

**Files:**
- Modify: `internal/config/settings_test.go`
- Modify: `internal/automation/pipeline_test.go`

- [ ] **Step 1: Check existing settings tests for the new field**

Read `internal/config/settings_test.go` and `internal/automation/pipeline_test.go` to find where automation fields are asserted. Add `SetViteTLS` assertions alongside the existing `SetAppURL` assertions.

Search for patterns like `a.SetAppURL` in the test files and add matching `a.SetViteTLS` checks. Also search for `{"set_app_url"` in pipeline tests and add `{"set_vite_tls", a.SetViteTLS}`.

- [ ] **Step 2: Run tests**

Run: `go test ./internal/config/ ./internal/automation/ -v`
Expected: All PASS.

- [ ] **Step 3: Commit**

```bash
git add internal/config/settings_test.go internal/automation/pipeline_test.go
git commit -m "Add SetViteTLS to settings and pipeline tests"
```

### Task 6: Final verification

- [ ] **Step 1: Run full test suite**

Run: `go test ./...`
Expected: All PASS.

- [ ] **Step 2: Run lint**

Run: `gofmt -l internal/certs/ internal/laravel/ internal/config/ internal/automation/ cmd/ && go vet ./...`
Expected: Clean.

- [ ] **Step 3: Verify build**

Run: `go build -o /dev/null .`
Expected: Builds successfully.

- [ ] **Step 4: Verify no remaining Valet references in source**

Run: `grep -r "ValetConfig\|ValetCerts\|config/valet\|RemoveConfig" internal/ cmd/ --include="*.go" | grep -v _test.go | grep -v plans/ | grep -v specs/`
Expected: No matches (all Valet references removed from non-test source files).
