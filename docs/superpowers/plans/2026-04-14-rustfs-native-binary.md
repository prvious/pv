# RustFS Native Binary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Docker-based S3 service with RustFS supervised as a native binary by the pv daemon, and establish the `BinaryService` + `supervisor` infrastructure that mail and future services will reuse.

**Architecture:** New `BinaryService` interface parallel to the existing Docker `Service` interface. New `supervisor` package spawns/watches/restarts child processes. The existing `ServerManager.Reconcile()` gains a binary-service phase that diffs registry state against supervisor state. Binary-service commands write state → `server.SignalDaemon()` → daemon reconciles in place. No new IPC.

**Tech Stack:** Go, cobra, existing pv patterns (binaries package, services package, registry, ServerManager SIGHUP/reconcile from `2026-03-27-server-reconcile-design.md`). `archive/zip` stdlib. No new external dependencies.

**Spec:** `docs/superpowers/specs/2026-04-14-rustfs-native-binary-design.md`

---

## File Structure

| Path | Action | Responsibility |
|------|--------|---------------|
| `internal/binaries/rustfs.go` | Create | Platform-specific archive name + download URL for RustFS |
| `internal/binaries/rustfs_test.go` | Create | URL construction tests |
| `internal/binaries/manager.go` | Modify | Add `rustfs` cases to `DownloadURL` and `LatestVersionURL` |
| `internal/registry/registry.go` | Modify | Add `Kind` and `Enabled` fields to `ServiceInstance` |
| `internal/registry/registry_test.go` | Modify | Add JSON round-trip tests for new fields |
| `internal/services/binary.go` | Create | `BinaryService` interface, `ReadyCheck` type, `binaryRegistry`, `LookupBinary`, `AllBinary` |
| `internal/services/binary_test.go` | Create | Tests for `LookupBinary`, `AllBinary` |
| `internal/services/rustfs.go` | Create | `RustFS` struct implementing `BinaryService`, registered as `"s3"` |
| `internal/services/rustfs_test.go` | Create | Method-output tests for `RustFS` |
| `internal/services/service.go` | Modify | Remove `"s3"` from Docker `registry` map; `Available()` to include binary names |
| `internal/services/s3.go` | Delete | Old Docker S3 implementation replaced by `rustfs.go` |
| `internal/services/s3_test.go` | Delete | Tests for deleted struct |
| `internal/supervisor/supervisor.go` | Create | `Process` struct, `Supervisor` with `Start`/`Stop`/`StopAll`/`IsRunning`/`Pid`/`SupervisedNames` |
| `internal/supervisor/supervisor_test.go` | Create | Tests using `/bin/sh` and `/bin/sleep` as test subjects |
| `internal/server/binary_service.go` | Create | `buildSupervisorProcess(BinaryService)` helper; `writeDaemonStatus(*Supervisor)` |
| `internal/server/binary_service_test.go` | Create | Tests for helper functions |
| `internal/server/manager.go` | Modify | `ServerManager` gains `supervisor` field; `Reconcile()` extended; `Shutdown()` calls `supervisor.StopAll`; `NewServerManager` signature changes |
| `internal/server/manager_test.go` | Modify | New test cases for binary-service reconciliation |
| `internal/server/process.go` | Modify | `Start()` creates supervisor + passes to manager; filter binary services from `bootColimaAndRecover` trigger |
| `internal/colima/recovery.go` | Modify | `ServicesToRecover` filters by `Kind != "binary"` |
| `internal/commands/service/dispatch.go` | Create | `resolveKind` helper |
| `internal/commands/service/dispatch_test.go` | Create | Dispatcher tests |
| `internal/commands/service/add.go` | Modify | Dispatch on kind; binary path |
| `internal/commands/service/start.go` | Modify | Dispatch on kind; binary path |
| `internal/commands/service/stop.go` | Modify | Dispatch on kind; binary path |
| `internal/commands/service/remove.go` | Modify | Dispatch on kind; binary path |
| `internal/commands/service/destroy.go` | Modify | Dispatch on kind; binary path |
| `internal/commands/service/status.go` | Modify | Read `daemon-status.json` for binary services |
| `internal/commands/service/list.go` | Modify | Merged table across both kinds |
| `internal/commands/service/logs.go` | Modify | Tail log file for binary services |
| `cmd/update.go` | Modify | Iterate `services.AllBinary()` for version refresh |
| `scripts/e2e/s3-binary.sh` | Create | E2E flow covering the complete lifecycle |
| `.github/workflows/e2e.yml` | Modify | Add phase that runs `scripts/e2e/s3-binary.sh` |

---

## Task 1: Verify RustFS distribution

**Files:**
- None modified in this task. Research-only.

No placeholder assumptions ship into code. This task produces facts the next tasks depend on.

- [ ] **Step 1: Query GitHub releases API for asset list**

```bash
curl -s https://api.github.com/repos/rustfs/rustfs/releases/latest | \
  python3 -c "import json,sys; d=json.load(sys.stdin); print('tag:', d['tag_name']); [print(' -', a['name']) for a in d['assets']]"
```

Record the output. Expected: a tag like `1.0.0-alpha.XX` and asset names matching the pattern `rustfs-<platform>-latest.zip`.

- [ ] **Step 2: Download the macOS arm64 asset and inspect**

```bash
cd /tmp
TAG=$(curl -s https://api.github.com/repos/rustfs/rustfs/releases/latest | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')
curl -L -o rustfs-test.zip "https://github.com/rustfs/rustfs/releases/download/${TAG}/rustfs-macos-aarch64-latest.zip"
unzip -l rustfs-test.zip
```

Record whether `rustfs` is at the root of the zip or nested in a subdirectory.

- [ ] **Step 3: Extract and check flag syntax**

```bash
unzip -o rustfs-test.zip -d /tmp/rustfs-extract
chmod +x /tmp/rustfs-extract/rustfs
/tmp/rustfs-extract/rustfs --help 2>&1 | head -60
/tmp/rustfs-extract/rustfs server --help 2>&1 | head -60
```

Confirm:
- Does `server` accept a positional data-dir argument?
- What are the flags for API port and console port? (Spec assumes `--address :9000 --console-address :9001`)

- [ ] **Step 4: Update the spec if reality diverges**

If any assumption in `docs/superpowers/specs/2026-04-14-rustfs-native-binary-design.md` is wrong (flag names, archive layout, platform asset names), amend the spec in a small commit before proceeding. Example:

```bash
git add docs/superpowers/specs/2026-04-14-rustfs-native-binary-design.md
git commit -m "Fix rustfs spec: correct flag names from --address to -A"
```

- [ ] **Step 5: Record verification results in a notes file (not committed)**

Write to `/tmp/rustfs-verification.md` a concise record of:
- Tag name tested
- Extracted binary path(s)
- Verified flag names
- Asset filenames for the three unverified platforms (darwin/amd64, linux/amd64, linux/arm64) as listed by the GitHub API

Subsequent tasks will reference this file.

---

## Task 2: Rustfs Binary descriptor

**Files:**
- Create: `internal/binaries/rustfs.go`
- Create: `internal/binaries/rustfs_test.go`
- Modify: `internal/binaries/manager.go`

- [ ] **Step 1: Write the failing tests**

Create `internal/binaries/rustfs_test.go`:

```go
package binaries

import (
	"runtime"
	"strings"
	"testing"
)

func TestRustfsURL_CurrentPlatform(t *testing.T) {
	url, err := rustfsURL("1.0.0-alpha.93")
	if err != nil {
		t.Fatalf("unexpected error for %s/%s: %v", runtime.GOOS, runtime.GOARCH, err)
	}
	if !strings.HasPrefix(url, "https://github.com/rustfs/rustfs/releases/download/1.0.0-alpha.93/") {
		t.Errorf("URL = %q; missing expected prefix", url)
	}
	if !strings.HasSuffix(url, ".zip") {
		t.Errorf("URL = %q; expected .zip suffix", url)
	}
}

func TestRustfsArchiveName_AllPlatforms(t *testing.T) {
	tests := []struct {
		goos, goarch, want string
	}{
		{"darwin", "arm64", "rustfs-macos-aarch64-latest.zip"},
		{"darwin", "amd64", "rustfs-macos-x86_64-latest.zip"},
		{"linux", "amd64", "rustfs-linux-x86_64-latest.zip"},
		{"linux", "arm64", "rustfs-linux-aarch64-latest.zip"},
	}
	for _, tc := range tests {
		archMap, ok := rustfsPlatformNames[tc.goos]
		if !ok {
			t.Errorf("no entry for GOOS=%s", tc.goos)
			continue
		}
		platform, ok := archMap[tc.goarch]
		if !ok {
			t.Errorf("no entry for GOARCH=%s on %s", tc.goarch, tc.goos)
			continue
		}
		got := "rustfs-" + platform + "-latest.zip"
		if got != tc.want {
			t.Errorf("%s/%s: got %q, want %q", tc.goos, tc.goarch, got, tc.want)
		}
	}
}

func TestDownloadURL_RustfsCase(t *testing.T) {
	url, err := DownloadURL(Rustfs, "1.0.0-alpha.93")
	if err != nil {
		t.Fatalf("DownloadURL returned error: %v", err)
	}
	if url == "" {
		t.Error("DownloadURL returned empty string")
	}
}

func TestLatestVersionURL_RustfsCase(t *testing.T) {
	got := LatestVersionURL(Rustfs)
	want := "https://api.github.com/repos/rustfs/rustfs/releases/latest"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/binaries/ -run Rustfs -v
```

Expected: FAIL — `undefined: rustfsURL`, `undefined: rustfsPlatformNames`, `undefined: Rustfs`.

- [ ] **Step 3: Create the descriptor file**

Create `internal/binaries/rustfs.go`:

```go
package binaries

import (
	"fmt"
	"runtime"
)

var Rustfs = Binary{
	Name:         "rustfs",
	DisplayName:  "RustFS",
	NeedsExtract: true,
}

var rustfsPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "macos-aarch64",
		"amd64": "macos-x86_64",
	},
	"linux": {
		"amd64": "linux-x86_64",
		"arm64": "linux-aarch64",
	},
}

func rustfsArchiveName() (string, error) {
	archMap, ok := rustfsPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for RustFS: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for RustFS: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("rustfs-%s-latest.zip", platform), nil
}

func rustfsURL(version string) (string, error) {
	archive, err := rustfsArchiveName()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://github.com/rustfs/rustfs/releases/download/%s/%s", version, archive), nil
}
```

- [ ] **Step 4: Wire into manager.go**

Edit `internal/binaries/manager.go`. Add `case "rustfs":` branches.

In `DownloadURL`, before `default:`:

```go
case "rustfs":
    return rustfsURL(version)
```

In `LatestVersionURL`, before `default:`:

```go
case "rustfs":
    return "https://api.github.com/repos/rustfs/rustfs/releases/latest"
```

Do **not** add `Rustfs` to `Tools()` — it's a backing service, not a user-exposed tool.

- [ ] **Step 5: Run tests to verify they pass**

```bash
gofmt -w internal/binaries/
go vet ./internal/binaries/
go test ./internal/binaries/ -v
```

Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add internal/binaries/rustfs.go internal/binaries/rustfs_test.go internal/binaries/manager.go
git commit -m "Add Rustfs binary descriptor

Mirror the Mago/Composer binary-descriptor pattern for RustFS.
Download URL + latest-version URL wired through manager.go.
Rustfs is not added to Tools() because it is a backing service,
not a user-facing CLI tool."
```

---

## Task 2b: `ExtractZip` helper and `installRustfs` function

**Files:**
- Modify: `internal/binaries/download.go` (add `ExtractZip`)
- Modify: `internal/binaries/install.go` (add `installRustfs` + `case "rustfs":` in `InstallBinary` switch)
- Create: `internal/binaries/download_zip_test.go`

**Real API reference (already verified in the existing code):**
- `ExtractTarGz(archivePath, destPath, binaryName string) error` — in `download.go`. Extracts a single named binary from a `.tar.gz`, writes to `destPath`.
- `MakeExecutable(path string) error` — in `download.go`.
- `InstallBinary(client *http.Client, b Binary, version string) error` — in `install.go`. Switches on `b.Name` to dispatch to `installMago` or `installComposer`. We add a `"rustfs"` case.
- `DownloadProgress(client *http.Client, url, destPath string, progress ProgressFunc) error` — in `download.go`.

The `installMago` function is our template:

```go
func installMago(client *http.Client, url string, progress ProgressFunc) error {
	internalBin := config.InternalBinDir()
	archivePath := filepath.Join(internalBin, "mago.tar.gz")
	destPath := filepath.Join(internalBin, "mago")

	if err := DownloadProgress(client, url, archivePath, progress); err != nil {
		return err
	}
	if err := ExtractTarGz(archivePath, destPath, "mago"); err != nil {
		return err
	}
	os.Remove(archivePath)
	return MakeExecutable(destPath)
}
```

- [ ] **Step 1: Write the failing test for `ExtractZip`**

Create `internal/binaries/download_zip_test.go`:

```go
package binaries

import (
	"archive/zip"
	"os"
	"path/filepath"
	"testing"
)

func TestExtractZip_FlattensSingleBinary(t *testing.T) {
	tmp := t.TempDir()
	zipPath := filepath.Join(tmp, "test.zip")
	f, err := os.Create(zipPath)
	if err != nil {
		t.Fatal(err)
	}
	w := zip.NewWriter(f)
	fw, err := w.Create("nested/dir/rustfs")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := fw.Write([]byte("#!/bin/sh\necho hi\n")); err != nil {
		t.Fatal(err)
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}

	destPath := filepath.Join(tmp, "out", "rustfs")
	os.MkdirAll(filepath.Dir(destPath), 0o755)
	if err := ExtractZip(zipPath, destPath, "rustfs"); err != nil {
		t.Fatal(err)
	}

	info, err := os.Stat(destPath)
	if err != nil {
		t.Fatalf("expected %s to exist: %v", destPath, err)
	}
	if info.Mode().Perm()&0o100 == 0 {
		t.Errorf("expected %s to be executable, got mode %v", destPath, info.Mode())
	}
}

func TestExtractZip_MissingBinary(t *testing.T) {
	tmp := t.TempDir()
	zipPath := filepath.Join(tmp, "test.zip")
	f, _ := os.Create(zipPath)
	w := zip.NewWriter(f)
	fw, _ := w.Create("something-else")
	fw.Write([]byte("x"))
	w.Close()
	f.Close()

	err := ExtractZip(zipPath, filepath.Join(tmp, "out"), "rustfs")
	if err == nil {
		t.Fatal("expected error when binary not found in zip")
	}
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
go test ./internal/binaries/ -run ExtractZip -v
```

Expected: FAIL — `undefined: ExtractZip`.

- [ ] **Step 3: Implement `ExtractZip` in `download.go`**

Append to `internal/binaries/download.go`:

```go
// ExtractZip extracts a single binary from a .zip archive at archivePath,
// locating the file by basename and writing it to destPath with 0o755 mode.
// Mirrors the semantics of ExtractTarGz for .zip archives.
func ExtractZip(archivePath, destPath, binaryName string) error {
	r, err := zip.OpenReader(archivePath)
	if err != nil {
		return fmt.Errorf("open zip %s: %w", archivePath, err)
	}
	defer r.Close()

	if err := os.MkdirAll(filepath.Dir(destPath), 0o755); err != nil {
		return err
	}

	for _, f := range r.File {
		if f.FileInfo().IsDir() {
			continue
		}
		if filepath.Base(f.Name) != binaryName {
			continue
		}
		rc, err := f.Open()
		if err != nil {
			return fmt.Errorf("open %s in zip: %w", f.Name, err)
		}
		out, err := os.OpenFile(destPath, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o755)
		if err != nil {
			rc.Close()
			return fmt.Errorf("create %s: %w", destPath, err)
		}
		_, copyErr := io.Copy(out, rc)
		rc.Close()
		out.Close()
		if copyErr != nil {
			return fmt.Errorf("copy %s: %w", f.Name, copyErr)
		}
		return nil
	}
	return fmt.Errorf("binary %q not found in zip %s", binaryName, archivePath)
}
```

Add `"archive/zip"` to the imports of `download.go`.

- [ ] **Step 4: Run the ExtractZip test to verify pass**

```bash
gofmt -w internal/binaries/
go vet ./internal/binaries/
go test ./internal/binaries/ -run ExtractZip -v
```

Expected: PASS.

- [ ] **Step 5: Add `installRustfs` and wire into `InstallBinary`**

Edit `internal/binaries/install.go`. Add a new function, mirroring `installMago`:

```go
func installRustfs(client *http.Client, url string, progress ProgressFunc) error {
	internalBin := config.InternalBinDir()
	archivePath := filepath.Join(internalBin, "rustfs.zip")
	destPath := filepath.Join(internalBin, "rustfs")

	if err := DownloadProgress(client, url, archivePath, progress); err != nil {
		return err
	}
	if err := ExtractZip(archivePath, destPath, "rustfs"); err != nil {
		return err
	}
	os.Remove(archivePath)
	return MakeExecutable(destPath)
}
```

Update the switch in `InstallBinaryProgress`:

```go
switch b.Name {
case "mago":
	return installMago(client, url, progress)
case "composer":
	return installComposer(client, url, b, version, progress)
case "rustfs":
	return installRustfs(client, url, progress)
default:
	return fmt.Errorf("unknown binary: %s", b.Name)
}
```

- [ ] **Step 6: Build, vet, test**

```bash
gofmt -w internal/binaries/
go vet ./internal/binaries/
go test ./internal/binaries/ -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add internal/binaries/
git commit -m "Add ExtractZip + installRustfs for RustFS binaries

ExtractZip mirrors ExtractTarGz for the zip format used by RustFS
releases. installRustfs follows the installMago pattern: download
the .zip, extract the binary, delete the archive, chmod 0755.
Wired into InstallBinary via a new rustfs case."
```

---

## Task 3: Registry `Kind` + `Enabled` fields

**Files:**
- Modify: `internal/registry/registry.go` (lines 13-17)
- Modify: `internal/registry/registry_test.go`

- [ ] **Step 1: Write the failing tests**

Append to `internal/registry/registry_test.go`:

```go
func TestServiceInstance_JSON_WithKindEnabled(t *testing.T) {
	enabled := true
	si := ServiceInstance{
		Image:       "",
		Port:        9000,
		ConsolePort: 9001,
		Kind:        "binary",
		Enabled:     &enabled,
	}
	data, err := json.Marshal(si)
	if err != nil {
		t.Fatal(err)
	}
	var back ServiceInstance
	if err := json.Unmarshal(data, &back); err != nil {
		t.Fatal(err)
	}
	if back.Kind != "binary" {
		t.Errorf("Kind round-trip: got %q", back.Kind)
	}
	if back.Enabled == nil || *back.Enabled != true {
		t.Errorf("Enabled round-trip: got %v", back.Enabled)
	}
}

func TestServiceInstance_JSON_OldFormat_DefaultsToDocker(t *testing.T) {
	// Entries written by earlier pv versions do not include Kind or Enabled.
	blob := []byte(`{"image":"redis:7","port":6379}`)
	var si ServiceInstance
	if err := json.Unmarshal(blob, &si); err != nil {
		t.Fatal(err)
	}
	if si.Kind != "" {
		t.Errorf("old entry should deserialize with empty Kind; got %q", si.Kind)
	}
	if si.Enabled != nil {
		t.Errorf("old entry should deserialize with nil Enabled; got %v", si.Enabled)
	}
}

func TestServiceInstance_JSON_EmptyFields_Omitted(t *testing.T) {
	si := ServiceInstance{Image: "redis:7", Port: 6379}
	data, _ := json.Marshal(si)
	s := string(data)
	if strings.Contains(s, "kind") {
		t.Errorf("expected kind to be omitted when empty; got %s", s)
	}
	if strings.Contains(s, "enabled") {
		t.Errorf("expected enabled to be omitted when nil; got %s", s)
	}
}
```

If `encoding/json` and `strings` aren't already imported in the file, add them.

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/registry/ -run ServiceInstance_JSON -v
```

Expected: FAIL — `unknown field Kind in struct literal of type ServiceInstance`.

- [ ] **Step 3: Add fields to `ServiceInstance`**

Edit `internal/registry/registry.go` lines 13-17. Replace:

```go
type ServiceInstance struct {
	Image       string `json:"image"`
	Port        int    `json:"port"`
	ConsolePort int    `json:"console_port,omitempty"`
}
```

With:

```go
type ServiceInstance struct {
	Image       string `json:"image,omitempty"`
	Port        int    `json:"port"`
	ConsolePort int    `json:"console_port,omitempty"`
	// Kind is "docker" (default) or "binary". Empty/unset is treated as "docker"
	// for backwards compatibility with registry files written by earlier pv versions.
	Kind string `json:"kind,omitempty"`
	// Enabled is only meaningful for Kind == "binary". nil is treated as enabled=true
	// (same back-compat reason). A non-nil false means "registered but stopped".
	Enabled *bool `json:"enabled,omitempty"`
}
```

Note: `Image` moved from `json:"image"` to `json:"image,omitempty"` so binary services don't emit an empty `"image": ""` string.

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w internal/registry/
go vet ./internal/registry/
go test ./internal/registry/ -v
```

Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/registry/registry.go internal/registry/registry_test.go
git commit -m "Add Kind and Enabled fields to registry.ServiceInstance

Kind is \"docker\" (default) or \"binary\". Enabled is only used for
binary services; nil means enabled=true so existing entries written
by older pv versions keep working unchanged. Image field gains
omitempty so binary entries do not emit an empty image string."
```

---

## Task 4: `BinaryService` interface + registry scaffolding

**Files:**
- Create: `internal/services/binary.go`
- Create: `internal/services/binary_test.go`

- [ ] **Step 1: Write the failing tests**

Create `internal/services/binary_test.go`:

```go
package services

import "testing"

func TestLookupBinary_Unknown_ReturnsFalse(t *testing.T) {
	_, ok := LookupBinary("does-not-exist")
	if ok {
		t.Error("expected ok=false for unknown name")
	}
}

func TestLookupBinary_KnownRegistered(t *testing.T) {
	// This test is populated by Task 5 when RustFS is registered.
	// For now we just assert the function exists and returns the empty-map result.
	if binaryRegistry == nil {
		t.Fatal("binaryRegistry should not be nil")
	}
}

func TestAllBinary_ReturnsRegistryMap(t *testing.T) {
	m := AllBinary()
	if m == nil {
		t.Error("AllBinary should not return nil")
	}
	// Identity equality is not guaranteed by the interface, but content equality is.
	if len(m) != len(binaryRegistry) {
		t.Errorf("AllBinary size %d != binaryRegistry size %d", len(m), len(binaryRegistry))
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/services/ -run Binary -v
```

Expected: FAIL — `undefined: LookupBinary`, `undefined: binaryRegistry`, `undefined: AllBinary`.

- [ ] **Step 3: Create the interface file**

Create `internal/services/binary.go`:

```go
package services

import (
	"time"

	"github.com/prvious/pv/internal/binaries"
)

// ReadyCheck describes how a supervisor verifies that a binary service has
// finished starting and is ready to accept requests. Exactly one of TCPPort
// or HTTPEndpoint must be set.
type ReadyCheck struct {
	TCPPort      int           // probe 127.0.0.1:port until Dial succeeds
	HTTPEndpoint string        // GET this URL, expect a 2xx response
	Timeout      time.Duration // overall give-up time for the ready probe
}

// BinaryService is the contract for services that run as native binaries
// supervised by the pv daemon, rather than as Docker containers.
type BinaryService interface {
	Name() string
	DisplayName() string

	// Binary returns the binaries.Binary descriptor that owns download / URL logic.
	Binary() binaries.Binary

	// Args returns CLI args passed to the binary at spawn time.
	// dataDir is the absolute path to this service's persistent data directory.
	Args(dataDir string) []string

	// Env returns process env vars to add on top of os.Environ().
	Env() []string

	// Port is the primary service port exposed on 127.0.0.1.
	Port() int

	// ConsolePort is a secondary port (admin UI), or 0 if none.
	ConsolePort() int

	// WebRoutes exposes HTTP subdomains (e.g. s3.pv.test -> 9001) to FrankenPHP.
	WebRoutes() []WebRoute

	// EnvVars returns the env vars injected into a linked project's .env file.
	EnvVars(projectName string) map[string]string

	// ReadyCheck describes how to verify the spawned process is accepting requests.
	ReadyCheck() ReadyCheck
}

// binaryRegistry is populated by init() functions in per-service files
// (e.g. rustfs.go registers itself as "s3").
var binaryRegistry = map[string]BinaryService{}

// LookupBinary returns the BinaryService registered under name, or ok=false.
func LookupBinary(name string) (BinaryService, bool) {
	svc, ok := binaryRegistry[name]
	return svc, ok
}

// AllBinary returns a snapshot of the binary-service registry.
// Callers must not mutate the returned map.
func AllBinary() map[string]BinaryService {
	out := make(map[string]BinaryService, len(binaryRegistry))
	for k, v := range binaryRegistry {
		out[k] = v
	}
	return out
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w internal/services/
go vet ./internal/services/
go test ./internal/services/ -run Binary -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/services/binary.go internal/services/binary_test.go
git commit -m "Add BinaryService interface and binary registry

Parallel abstraction to the existing Docker Service interface.
Registry is populated by init() functions in per-service files;
for now it stays empty. LookupBinary / AllBinary match the
existing Service API surface."
```

---

## Task 5: `RustFS` implementation, replacing Docker S3

**Files:**
- Create: `internal/services/rustfs.go`
- Create: `internal/services/rustfs_test.go`
- Delete: `internal/services/s3.go`
- Delete: `internal/services/s3_test.go`
- Modify: `internal/services/service.go` (remove `"s3"` from `registry` and update `Available()`)

- [ ] **Step 1: Write the failing tests**

Create `internal/services/rustfs_test.go`:

```go
package services

import (
	"reflect"
	"testing"
)

func TestRustFS_RegisteredAsS3(t *testing.T) {
	svc, ok := LookupBinary("s3")
	if !ok {
		t.Fatal("LookupBinary(\"s3\") returned ok=false")
	}
	if _, isRustfs := svc.(*RustFS); !isRustfs {
		t.Errorf("expected *RustFS, got %T", svc)
	}
}

func TestRustFS_Name(t *testing.T) {
	r := &RustFS{}
	if r.Name() != "s3" {
		t.Errorf("Name() = %q, want s3", r.Name())
	}
}

func TestRustFS_Ports(t *testing.T) {
	r := &RustFS{}
	if r.Port() != 9000 {
		t.Errorf("Port() = %d, want 9000", r.Port())
	}
	if r.ConsolePort() != 9001 {
		t.Errorf("ConsolePort() = %d, want 9001", r.ConsolePort())
	}
}

func TestRustFS_WebRoutes(t *testing.T) {
	r := &RustFS{}
	want := []WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
	got := r.WebRoutes()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("WebRoutes() = %#v, want %#v", got, want)
	}
}

func TestRustFS_EnvVars_MatchesDockerKeys(t *testing.T) {
	// Linked projects rely on these exact .env keys; the binary migration
	// must not silently change them.
	r := &RustFS{}
	vars := r.EnvVars("myproject")
	wantKeys := []string{
		"AWS_ACCESS_KEY_ID",
		"AWS_SECRET_ACCESS_KEY",
		"AWS_DEFAULT_REGION",
		"AWS_BUCKET",
		"AWS_ENDPOINT",
		"AWS_USE_PATH_STYLE_ENDPOINT",
	}
	for _, k := range wantKeys {
		if _, ok := vars[k]; !ok {
			t.Errorf("EnvVars missing key %q", k)
		}
	}
	if vars["AWS_BUCKET"] != "myproject" {
		t.Errorf("AWS_BUCKET = %q, want myproject", vars["AWS_BUCKET"])
	}
	if vars["AWS_ENDPOINT"] != "http://127.0.0.1:9000" {
		t.Errorf("AWS_ENDPOINT = %q, want http://127.0.0.1:9000", vars["AWS_ENDPOINT"])
	}
}

func TestRustFS_Args_IncludesDataDir(t *testing.T) {
	r := &RustFS{}
	args := r.Args("/tmp/data")
	found := false
	for _, a := range args {
		if a == "/tmp/data" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("Args() did not include the data dir; got %v", args)
	}
}

func TestRustFS_ReadyCheck_TCP9000(t *testing.T) {
	r := &RustFS{}
	rc := r.ReadyCheck()
	if rc.TCPPort != 9000 {
		t.Errorf("ReadyCheck.TCPPort = %d, want 9000", rc.TCPPort)
	}
	if rc.HTTPEndpoint != "" {
		t.Errorf("ReadyCheck.HTTPEndpoint = %q, want empty (TCP probe)", rc.HTTPEndpoint)
	}
	if rc.Timeout == 0 {
		t.Error("ReadyCheck.Timeout must be non-zero")
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/services/ -run RustFS -v
```

Expected: FAIL — `undefined: RustFS`.

- [ ] **Step 3: Create the implementation**

Create `internal/services/rustfs.go`:

```go
package services

import (
	"time"

	"github.com/prvious/pv/internal/binaries"
)

type RustFS struct{}

// Adjust flags in Args() if Task 1 verification revealed different syntax.
func (r *RustFS) Name() string        { return "s3" }
func (r *RustFS) DisplayName() string { return "S3 Storage (RustFS)" }

func (r *RustFS) Binary() binaries.Binary { return binaries.Rustfs }

func (r *RustFS) Args(dataDir string) []string {
	return []string{
		"server", dataDir,
		"--address", ":9000",
		"--console-address", ":9001",
	}
}

func (r *RustFS) Env() []string {
	return []string{
		"RUSTFS_ROOT_USER=rstfsadmin",
		"RUSTFS_ROOT_PASSWORD=rstfsadmin",
	}
}

func (r *RustFS) Port() int        { return 9000 }
func (r *RustFS) ConsolePort() int { return 9001 }

func (r *RustFS) WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "s3", Port: 9001},
		{Subdomain: "s3-api", Port: 9000},
	}
}

func (r *RustFS) EnvVars(projectName string) map[string]string {
	return map[string]string{
		"AWS_ACCESS_KEY_ID":           "rstfsadmin",
		"AWS_SECRET_ACCESS_KEY":       "rstfsadmin",
		"AWS_DEFAULT_REGION":          "us-east-1",
		"AWS_BUCKET":                  projectName,
		"AWS_ENDPOINT":                "http://127.0.0.1:9000",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
	}
}

func (r *RustFS) ReadyCheck() ReadyCheck {
	return ReadyCheck{
		TCPPort: 9000,
		Timeout: 30 * time.Second,
	}
}

func init() {
	binaryRegistry["s3"] = &RustFS{}
}
```

- [ ] **Step 4: Delete the Docker S3 implementation**

```bash
git rm internal/services/s3.go internal/services/s3_test.go
```

- [ ] **Step 5: Remove `"s3"` from the Docker registry and update `Available()`**

Edit `internal/services/service.go`:

Before:
```go
var registry = map[string]Service{
	"mail":     &Mail{},
	"mysql":    &MySQL{},
	"postgres": &Postgres{},
	"redis":    &Redis{},
	"s3":       &S3{},
}

func Lookup(name string) (Service, error) {
	svc, ok := registry[name]
	if !ok {
		return nil, fmt.Errorf("unknown service %q (available: mail, mysql, postgres, redis, s3)", name)
	}
	return svc, nil
}

func Available() []string {
	return []string{"mail", "mysql", "postgres", "redis", "s3"}
}
```

After:
```go
var registry = map[string]Service{
	"mail":     &Mail{},
	"mysql":    &MySQL{},
	"postgres": &Postgres{},
	"redis":    &Redis{},
}

func Lookup(name string) (Service, error) {
	svc, ok := registry[name]
	if !ok {
		return nil, fmt.Errorf("unknown service %q (available: %s)", name, strings.Join(Available(), ", "))
	}
	return svc, nil
}

// Available returns the union of Docker and binary service names, sorted.
func Available() []string {
	names := make([]string, 0, len(registry)+len(binaryRegistry))
	for n := range registry {
		names = append(names, n)
	}
	for n := range binaryRegistry {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}
```

Add `"sort"` and `"strings"` to the imports of `service.go` if not present.

- [ ] **Step 6: Run all package tests**

```bash
gofmt -w internal/services/
go vet ./internal/services/
go test ./internal/services/ -v
```

Expected: all PASS, including the lookup test that now finds `RustFS` under `"s3"`.

- [ ] **Step 7: Full-project build to catch any stale references**

```bash
go build ./...
```

Expected: build succeeds. If any file references the deleted `S3` type, fix those references — they should have been looking up the service through the services package interface, not the concrete type.

- [ ] **Step 8: Commit**

```bash
git add internal/services/rustfs.go internal/services/rustfs_test.go internal/services/service.go
git add -u internal/services/s3.go internal/services/s3_test.go
git commit -m "Replace Docker S3 service with RustFS BinaryService

RustFS registers itself as \"s3\" in the binary registry. The old
Docker-backed S3 implementation is removed — there is no longer a
docker path for s3. Available() now returns the union of Docker
and binary service names so CLI help messages stay correct."
```

---

## Task 6: Supervisor package

**Files:**
- Create: `internal/supervisor/supervisor.go`
- Create: `internal/supervisor/supervisor_test.go`

- [ ] **Step 1: Write the failing tests**

Create `internal/supervisor/supervisor_test.go`:

```go
package supervisor

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// newTestProcess returns a Process that runs `sh -c <cmd>` and writes logs
// to a file inside t.TempDir(). Ready is a no-op (returns nil immediately).
func newTestProcess(t *testing.T, name, shellCmd string) Process {
	t.Helper()
	logPath := filepath.Join(t.TempDir(), name+".log")
	return Process{
		Name:    name,
		Binary:  "/bin/sh",
		Args:    []string{"-c", shellCmd},
		LogFile: logPath,
		Ready: func(ctx context.Context) error {
			return nil
		},
		ReadyTimeout: 2 * time.Second,
	}
}

func TestSupervisor_StartStop_Sleep(t *testing.T) {
	s := New()
	p := newTestProcess(t, "sleeper", "sleep 30")
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	if !s.IsRunning("sleeper") {
		t.Fatal("expected IsRunning=true after Start")
	}
	if s.Pid("sleeper") == 0 {
		t.Error("Pid should be non-zero after successful Start")
	}
	if err := s.Stop("sleeper", 2*time.Second); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if s.IsRunning("sleeper") {
		t.Error("expected IsRunning=false after Stop")
	}
}

func TestSupervisor_StopAll(t *testing.T) {
	s := New()
	for _, name := range []string{"a", "b", "c"} {
		if err := s.Start(context.Background(), newTestProcess(t, name, "sleep 30")); err != nil {
			t.Fatalf("Start %s: %v", name, err)
		}
	}
	if err := s.StopAll(2 * time.Second); err != nil {
		t.Fatalf("StopAll: %v", err)
	}
	for _, name := range []string{"a", "b", "c"} {
		if s.IsRunning(name) {
			t.Errorf("%s still running after StopAll", name)
		}
	}
}

func TestSupervisor_ReadyTimeout(t *testing.T) {
	s := New()
	p := newTestProcess(t, "never-ready", "sleep 30")
	p.Ready = func(ctx context.Context) error {
		return errors.New("not ready")
	}
	p.ReadyTimeout = 500 * time.Millisecond

	start := time.Now()
	err := s.Start(context.Background(), p)
	elapsed := time.Since(start)

	if err == nil {
		t.Fatal("expected ready-timeout error")
	}
	if elapsed > 2*time.Second {
		t.Errorf("Start took %v; expected close to ReadyTimeout", elapsed)
	}
	// The process should have been killed after the timeout.
	if s.IsRunning("never-ready") {
		t.Error("expected process to be stopped after ready timeout")
	}
}

func TestSupervisor_SupervisedNames(t *testing.T) {
	s := New()
	if len(s.SupervisedNames()) != 0 {
		t.Error("expected empty list on fresh Supervisor")
	}
	if err := s.Start(context.Background(), newTestProcess(t, "x", "sleep 30")); err != nil {
		t.Fatalf("Start: %v", err)
	}
	names := s.SupervisedNames()
	if len(names) != 1 || names[0] != "x" {
		t.Errorf("SupervisedNames = %v, want [x]", names)
	}
	if err := s.Stop("x", 2*time.Second); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if len(s.SupervisedNames()) != 0 {
		t.Error("expected empty list after Stop")
	}
}

func TestSupervisor_TCPReadyCheck(t *testing.T) {
	// Bind a TCP listener on a random port inside the test and use it as the
	// ready-check target to prove the dial-based ready logic works.
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	addr := ln.Addr().String()

	s := New()
	p := Process{
		Name:    "ready-tcp",
		Binary:  "/bin/sh",
		Args:    []string{"-c", "sleep 30"},
		LogFile: filepath.Join(t.TempDir(), "ready-tcp.log"),
		Ready: func(ctx context.Context) error {
			c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
			if err != nil {
				return err
			}
			c.Close()
			return nil
		},
		ReadyTimeout: 3 * time.Second,
	}
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	defer s.Stop("ready-tcp", 2*time.Second)
	if !s.IsRunning("ready-tcp") {
		t.Error("expected IsRunning=true after ready check passes")
	}
}

func TestSupervisor_LogFileIsWritten(t *testing.T) {
	s := New()
	p := newTestProcess(t, "logger", "echo hello-from-supervisor; sleep 30")
	if err := s.Start(context.Background(), p); err != nil {
		t.Fatalf("Start: %v", err)
	}
	defer s.Stop("logger", 2*time.Second)

	// Poll briefly for the log file to contain the expected line.
	deadline := time.Now().Add(2 * time.Second)
	var data []byte
	for time.Now().Before(deadline) {
		f, err := os.Open(p.LogFile)
		if err == nil {
			data, _ = io.ReadAll(f)
			f.Close()
			if len(data) > 0 {
				break
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	if len(data) == 0 {
		t.Fatalf("log file %s was empty after 2s", p.LogFile)
	}
	if !containsStr(string(data), "hello-from-supervisor") {
		t.Errorf("log file missing expected output; got %q", string(data))
	}
}

func containsStr(s, sub string) bool {
	return len(s) >= len(sub) && (fmt.Sprintf("%s", s)[0:] != "" && (indexOf(s, sub) >= 0))
}

func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}
```

(Intentionally not using `strings.Contains` to keep imports minimal and avoid a distraction when the standard library is available; feel free to switch to `strings.Contains` if preferred.)

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/supervisor/ -v
```

Expected: FAIL — `package supervisor does not exist`.

- [ ] **Step 3: Implement the supervisor**

Create `internal/supervisor/supervisor.go`:

```go
// Package supervisor spawns and watches child binary processes for the pv
// daemon. Each Process is launched, watched for liveness, restarted on crash
// (up to a budget), and stopped cleanly on demand. The supervisor runs
// in-process inside the pv daemon — it is not a standalone long-running
// process of its own.
package supervisor

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"syscall"
	"time"
)

// Process describes one supervised binary.
type Process struct {
	Name         string
	Binary       string   // absolute path to executable
	Args         []string
	Env          []string // appended to os.Environ()
	WorkingDir   string
	LogFile      string // absolute path; stdout+stderr appended here

	// Ready returns nil when the process is serving requests.
	Ready        func(ctx context.Context) error
	ReadyTimeout time.Duration
}

// managed holds the supervisor's internal state for a single Process.
type managed struct {
	proc     Process
	cmd      *exec.Cmd
	cancel   context.CancelFunc // cancels the watcher goroutine
	stopped  bool               // set true when Stop was called explicitly
	restarts []time.Time        // rolling window of restart timestamps
}

// Supervisor manages a set of child processes.
type Supervisor struct {
	mu        sync.Mutex
	processes map[string]*managed
}

// New constructs an empty supervisor.
func New() *Supervisor {
	return &Supervisor{processes: map[string]*managed{}}
}

// Start spawns p, waits for p.Ready to succeed, and returns.
// The process continues in the background; crashes are restarted
// according to the crash-budget policy.
func (s *Supervisor) Start(ctx context.Context, p Process) error {
	if p.Name == "" {
		return errors.New("supervisor: Process.Name is required")
	}

	// Pre-flight: ensure log file's parent directory exists.
	if p.LogFile != "" {
		if err := os.MkdirAll(filepath.Dir(p.LogFile), 0o755); err != nil {
			return fmt.Errorf("supervisor: create log dir: %w", err)
		}
	}

	s.mu.Lock()
	if _, exists := s.processes[p.Name]; exists {
		s.mu.Unlock()
		return fmt.Errorf("supervisor: %q is already supervised", p.Name)
	}
	s.mu.Unlock()

	m, err := s.spawn(p)
	if err != nil {
		return err
	}

	s.mu.Lock()
	s.processes[p.Name] = m
	s.mu.Unlock()

	// Start the watch goroutine so a crash is observed immediately.
	watchCtx, cancel := context.WithCancel(context.Background())
	m.cancel = cancel
	go s.watch(watchCtx, p.Name)

	// Ready-wait (blocks Start).
	if p.Ready != nil {
		if err := s.waitReady(ctx, p); err != nil {
			_ = s.Stop(p.Name, 5*time.Second)
			return fmt.Errorf("supervisor: %s not ready: %w", p.Name, err)
		}
	}
	return nil
}

// spawn opens the log file, builds the command, and starts it.
func (s *Supervisor) spawn(p Process) (*managed, error) {
	var logFile *os.File
	if p.LogFile != "" {
		f, err := os.OpenFile(p.LogFile, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
		if err != nil {
			return nil, fmt.Errorf("supervisor: open log: %w", err)
		}
		logFile = f
	}

	cmd := exec.Command(p.Binary, p.Args...)
	cmd.Env = append(os.Environ(), p.Env...)
	cmd.Dir = p.WorkingDir
	if logFile != nil {
		cmd.Stdout = logFile
		cmd.Stderr = logFile
	}

	if err := cmd.Start(); err != nil {
		if logFile != nil {
			_ = logFile.Close()
		}
		return nil, fmt.Errorf("supervisor: spawn %s: %w", p.Name, err)
	}
	return &managed{proc: p, cmd: cmd}, nil
}

// waitReady polls p.Ready every 250ms until success or timeout.
func (s *Supervisor) waitReady(ctx context.Context, p Process) error {
	deadline := time.Now().Add(p.ReadyTimeout)
	if p.ReadyTimeout == 0 {
		deadline = time.Now().Add(30 * time.Second)
	}
	for {
		if err := p.Ready(ctx); err == nil {
			return nil
		}
		if time.Now().After(deadline) {
			return errors.New("ready-check timed out")
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(250 * time.Millisecond):
		}
	}
}

// watch blocks on cmd.Wait and handles crash restarts.
func (s *Supervisor) watch(ctx context.Context, name string) {
	for {
		s.mu.Lock()
		m, ok := s.processes[name]
		s.mu.Unlock()
		if !ok {
			return
		}

		waitErr := m.cmd.Wait()

		s.mu.Lock()
		if m.stopped {
			delete(s.processes, name)
			s.mu.Unlock()
			return
		}

		// Crash recovery: enforce the restart budget (5 within 60s).
		now := time.Now()
		cutoff := now.Add(-60 * time.Second)
		filtered := m.restarts[:0]
		for _, t := range m.restarts {
			if t.After(cutoff) {
				filtered = append(filtered, t)
			}
		}
		m.restarts = filtered
		if len(m.restarts) >= 5 {
			fmt.Fprintf(os.Stderr, "supervisor: %s exceeded restart budget (5/60s); giving up (last error: %v)\n", name, waitErr)
			delete(s.processes, name)
			s.mu.Unlock()
			return
		}
		m.restarts = append(m.restarts, now)
		s.mu.Unlock()

		// Pause briefly and respawn — no ready-wait on recovery.
		select {
		case <-ctx.Done():
			return
		case <-time.After(2 * time.Second):
		}
		newM, err := s.spawn(m.proc)
		if err != nil {
			fmt.Fprintf(os.Stderr, "supervisor: %s respawn failed: %v\n", name, err)
			s.mu.Lock()
			delete(s.processes, name)
			s.mu.Unlock()
			return
		}
		s.mu.Lock()
		newM.stopped = m.stopped
		newM.cancel = m.cancel
		newM.restarts = m.restarts
		s.processes[name] = newM
		s.mu.Unlock()
	}
}

// Stop sends SIGTERM, waits up to timeout, then SIGKILL.
// After Stop returns, IsRunning(name) is false.
func (s *Supervisor) Stop(name string, timeout time.Duration) error {
	s.mu.Lock()
	m, ok := s.processes[name]
	if !ok {
		s.mu.Unlock()
		return nil
	}
	m.stopped = true
	if m.cancel != nil {
		m.cancel()
	}
	pid := m.cmd.Process.Pid
	cmd := m.cmd
	s.mu.Unlock()

	if err := cmd.Process.Signal(syscall.SIGTERM); err != nil && !errors.Is(err, os.ErrProcessDone) {
		return fmt.Errorf("supervisor: SIGTERM %s (pid %d): %w", name, pid, err)
	}

	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()

	select {
	case <-done:
	case <-time.After(timeout):
		_ = cmd.Process.Kill()
		<-done
	}

	s.mu.Lock()
	delete(s.processes, name)
	s.mu.Unlock()
	return nil
}

// StopAll stops every supervised process in parallel.
// timeout is per-process, not total.
func (s *Supervisor) StopAll(timeout time.Duration) error {
	s.mu.Lock()
	names := make([]string, 0, len(s.processes))
	for n := range s.processes {
		names = append(names, n)
	}
	s.mu.Unlock()

	var wg sync.WaitGroup
	for _, n := range names {
		wg.Add(1)
		go func(name string) {
			defer wg.Done()
			_ = s.Stop(name, timeout)
		}(n)
	}
	wg.Wait()
	return nil
}

// IsRunning reports whether name is a supervised process with a live PID.
func (s *Supervisor) IsRunning(name string) bool {
	s.mu.Lock()
	m, ok := s.processes[name]
	s.mu.Unlock()
	if !ok {
		return false
	}
	// Signal 0 checks that the process exists without delivering a signal.
	return m.cmd.Process.Signal(syscall.Signal(0)) == nil
}

// Pid returns the current pid of name, or 0 if not supervised.
func (s *Supervisor) Pid(name string) int {
	s.mu.Lock()
	defer s.mu.Unlock()
	if m, ok := s.processes[name]; ok {
		return m.cmd.Process.Pid
	}
	return 0
}

// SupervisedNames returns the set of currently-supervised process names.
func (s *Supervisor) SupervisedNames() []string {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]string, 0, len(s.processes))
	for n := range s.processes {
		out = append(out, n)
	}
	return out
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w internal/supervisor/
go vet ./internal/supervisor/
go test ./internal/supervisor/ -v -count=1
```

Expected: all PASS. If a test hangs, SIGTERM the test runner — it likely means `StopAll` or `Stop` isn't delivering SIGTERM correctly.

- [ ] **Step 5: Commit**

```bash
git add internal/supervisor/
git commit -m "Add supervisor package for child-process management

Supervisor spawns binaries, waits for ReadyCheck, restarts on crash
(with a 5-in-60s budget), and stops cleanly with SIGTERM->SIGKILL.
Tests exercise the happy path plus ready-timeout, StopAll, TCP
readiness, and log-file writes using /bin/sh as a stand-in binary."
```

---

## Task 7: `buildSupervisorProcess` helper and `daemon-status.json`

**Files:**
- Create: `internal/server/binary_service.go`
- Create: `internal/server/binary_service_test.go`

- [ ] **Step 1: Write the failing tests**

Create `internal/server/binary_service_test.go`:

```go
package server

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/supervisor"
)

func TestBuildSupervisorProcess_RustFS(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	svc := &services.RustFS{}
	p, err := buildSupervisorProcess(svc)
	if err != nil {
		t.Fatalf("buildSupervisorProcess: %v", err)
	}
	if p.Name != "rustfs" {
		t.Errorf("Name = %q, want rustfs", p.Name)
	}
	if !strings.HasSuffix(p.Binary, "/internal/bin/rustfs") {
		t.Errorf("Binary = %q; should end with /internal/bin/rustfs", p.Binary)
	}
	if !strings.Contains(p.LogFile, "logs") || !strings.HasSuffix(p.LogFile, "/rustfs.log") {
		t.Errorf("LogFile = %q; expected ~/.pv/logs/rustfs.log", p.LogFile)
	}
	// Data dir should be created on the fly.
	dataDir := ""
	for i, a := range p.Args {
		if a == "server" && i+1 < len(p.Args) {
			dataDir = p.Args[i+1]
			break
		}
	}
	if dataDir == "" {
		t.Fatal("could not find data dir in Args")
	}
	if _, err := os.Stat(dataDir); err != nil {
		t.Errorf("data dir %s should exist: %v", dataDir, err)
	}
	if p.Ready == nil {
		t.Error("Ready closure must be set")
	}
	if p.ReadyTimeout == 0 {
		t.Error("ReadyTimeout must be non-zero")
	}
}

func TestWriteDaemonStatus_RoundTrip(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	s := supervisor.New()
	// No live processes — we just test the file write path.
	if err := writeDaemonStatus(s); err != nil {
		t.Fatalf("writeDaemonStatus: %v", err)
	}
	path := filepath.Join(os.Getenv("HOME"), ".pv", "daemon-status.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read daemon-status.json: %v", err)
	}
	var snap DaemonStatus
	if err := json.Unmarshal(data, &snap); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if snap.PID != os.Getpid() {
		t.Errorf("PID = %d, want %d", snap.PID, os.Getpid())
	}
	if snap.Supervised == nil {
		t.Error("Supervised map should be initialized")
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/server/ -run SupervisorProcess -v
go test ./internal/server/ -run DaemonStatus -v
```

Expected: FAIL — `undefined: buildSupervisorProcess`, `undefined: writeDaemonStatus`, `undefined: DaemonStatus`.

- [ ] **Step 3: Implement the helpers**

Create `internal/server/binary_service.go`:

```go
package server

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/services"
	"github.com/prvious/pv/internal/supervisor"
)

// DaemonStatus is the JSON snapshot written to ~/.pv/daemon-status.json.
type DaemonStatus struct {
	PID        int                         `json:"pid"`
	StartedAt  time.Time                   `json:"started_at"`
	Supervised map[string]SupervisedStatus `json:"supervised"`
}

type SupervisedStatus struct {
	PID     int  `json:"pid"`
	Running bool `json:"running"`
}

// daemonStartedAt is captured when the package is first initialized inside the
// daemon process. It's recorded in every status snapshot.
var daemonStartedAt = time.Now()

// buildSupervisorProcess translates a BinaryService into a supervisor.Process.
// It resolves all paths via internal/config and creates the data + log directories.
func buildSupervisorProcess(svc services.BinaryService) (supervisor.Process, error) {
	binaryName := svc.Binary().Name
	binaryPath := filepath.Join(config.InternalBinDir(), binaryName)

	dataDir := config.ServiceDataDir(svc.Name(), "latest")
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create data dir %s: %w", dataDir, err)
	}

	logFile := filepath.Join(config.PvDir(), "logs", binaryName+".log")
	if err := os.MkdirAll(filepath.Dir(logFile), 0o755); err != nil {
		return supervisor.Process{}, fmt.Errorf("create log dir: %w", err)
	}

	rc := svc.ReadyCheck()
	ready := buildReadyFunc(rc)

	return supervisor.Process{
		Name:         binaryName,
		Binary:       binaryPath,
		Args:         svc.Args(dataDir),
		Env:          svc.Env(),
		LogFile:      logFile,
		Ready:        ready,
		ReadyTimeout: rc.Timeout,
	}, nil
}

// buildReadyFunc returns a ReadyFunc appropriate to the ReadyCheck variant.
func buildReadyFunc(rc services.ReadyCheck) func(context.Context) error {
	switch {
	case rc.HTTPEndpoint != "":
		client := &http.Client{Timeout: 2 * time.Second}
		url := rc.HTTPEndpoint
		return func(ctx context.Context) error {
			req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
			if err != nil {
				return err
			}
			resp, err := client.Do(req)
			if err != nil {
				return err
			}
			defer resp.Body.Close()
			if resp.StatusCode >= 200 && resp.StatusCode < 300 {
				return nil
			}
			return fmt.Errorf("HTTP %s returned %d", url, resp.StatusCode)
		}
	case rc.TCPPort > 0:
		addr := fmt.Sprintf("127.0.0.1:%d", rc.TCPPort)
		return func(ctx context.Context) error {
			d := net.Dialer{Timeout: 500 * time.Millisecond}
			c, err := d.DialContext(ctx, "tcp", addr)
			if err != nil {
				return err
			}
			c.Close()
			return nil
		}
	default:
		// Degenerate case: no probe specified. Treat as instantly ready.
		return func(context.Context) error { return nil }
	}
}

// writeDaemonStatus serializes the current supervisor state to
// ~/.pv/daemon-status.json. Safe to call from the reconcile path.
func writeDaemonStatus(sup *supervisor.Supervisor) error {
	snap := DaemonStatus{
		PID:        os.Getpid(),
		StartedAt:  daemonStartedAt,
		Supervised: map[string]SupervisedStatus{},
	}
	if sup != nil {
		for _, name := range sup.SupervisedNames() {
			snap.Supervised[name] = SupervisedStatus{
				PID:     sup.Pid(name),
				Running: sup.IsRunning(name),
			}
		}
	}
	data, err := json.MarshalIndent(snap, "", "  ")
	if err != nil {
		return err
	}
	path := filepath.Join(config.PvDir(), "daemon-status.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, data, 0o644)
}

// ReadDaemonStatus returns the parsed ~/.pv/daemon-status.json, or nil+error if
// the file is missing or corrupt or if the recorded PID isn't alive.
func ReadDaemonStatus() (*DaemonStatus, error) {
	path := filepath.Join(config.PvDir(), "daemon-status.json")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var snap DaemonStatus
	if err := json.Unmarshal(data, &snap); err != nil {
		return nil, err
	}
	// Liveness check.
	if proc, err := os.FindProcess(snap.PID); err == nil {
		if err := proc.Signal(syscall.Signal(0)); err != nil {
			return nil, fmt.Errorf("daemon-status.json is stale (pid %d not alive)", snap.PID)
		}
	}
	return &snap, nil
}
```

Add `"syscall"` to imports.

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w internal/server/
go vet ./internal/server/
go test ./internal/server/ -run "SupervisorProcess|DaemonStatus" -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/server/binary_service.go internal/server/binary_service_test.go
git commit -m "Add supervisor-process builder and daemon-status.json writer

buildSupervisorProcess translates a BinaryService into a
supervisor.Process by resolving paths via internal/config and
creating data + log directories. writeDaemonStatus captures the
supervisor snapshot for CLI readers; ReadDaemonStatus rejects
stale files when the recorded PID is dead."
```

---

## Task 8: Extend `ServerManager` with supervisor + binary-service reconcile

**Files:**
- Modify: `internal/server/manager.go`
- Modify: `internal/server/manager_test.go`

- [ ] **Step 1: Write the failing tests**

Append to `internal/server/manager_test.go`:

```go
func TestReconcile_SpawnsBinaryServices(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	// Seed a registry with s3 as a binary service.
	enabled := true
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "binary", Port: 9000, ConsolePort: 9001, Enabled: &enabled},
		},
	}
	// Save via the package so later code paths that reload the registry see it.
	if err := reg.Save(); err != nil {
		t.Fatal(err)
	}

	// Put a fake "rustfs" binary in place so supervisor spawn doesn't ENOENT.
	binDir := config.InternalBinDir()
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		t.Fatal(err)
	}
	fakeBin := filepath.Join(binDir, "rustfs")
	if err := os.WriteFile(fakeBin, []byte("#!/bin/sh\nsleep 30\n"), 0o755); err != nil {
		t.Fatal(err)
	}

	// Construct the manager with a supervisor.
	sup := supervisor.New()
	m := &ServerManager{supervisor: sup, secondaries: map[string]*FrankenPHP{}}
	defer m.supervisor.StopAll(2 * time.Second)

	// The binary-service reconcile should start rustfs.
	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatalf("reconcileBinaryServices: %v", err)
	}
	if !sup.IsRunning("rustfs") {
		t.Error("expected rustfs to be supervised after reconcile")
	}
}

func TestReconcile_StopsDisabledBinaryServices(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	binDir := config.InternalBinDir()
	os.MkdirAll(binDir, 0o755)
	os.WriteFile(filepath.Join(binDir, "rustfs"), []byte("#!/bin/sh\nsleep 30\n"), 0o755)

	sup := supervisor.New()
	m := &ServerManager{supervisor: sup, secondaries: map[string]*FrankenPHP{}}

	// Phase 1: enabled, should start.
	enabled := true
	reg1 := &registry.Registry{Services: map[string]*registry.ServiceInstance{
		"s3": {Kind: "binary", Port: 9000, Enabled: &enabled},
	}}
	reg1.Save()
	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if !sup.IsRunning("rustfs") {
		t.Fatal("expected rustfs running after first reconcile")
	}

	// Phase 2: disabled, should stop.
	disabled := false
	reg2 := &registry.Registry{Services: map[string]*registry.ServiceInstance{
		"s3": {Kind: "binary", Port: 9000, Enabled: &disabled},
	}}
	reg2.Save()
	if err := m.reconcileBinaryServices(context.Background()); err != nil {
		t.Fatal(err)
	}
	if sup.IsRunning("rustfs") {
		t.Error("expected rustfs stopped after disabling via reconcile")
	}
}
```

Add imports for `context`, `os`, `path/filepath`, `time`, `internal/config`, `internal/registry`, `internal/supervisor` if not already present.

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/server/ -run "Reconcile_SpawnsBinaryServices|Reconcile_StopsDisabled" -v
```

Expected: FAIL — `unknown field supervisor`, `undefined: reconcileBinaryServices`.

- [ ] **Step 3: Modify the ServerManager struct and constructor**

Edit `internal/server/manager.go`.

Replace the existing `ServerManager` struct and `NewServerManager` function:

```go
type ServerManager struct {
	mu          sync.Mutex
	main        *FrankenPHP
	secondaries map[string]*FrankenPHP // PHP version -> instance
	supervisor  *Supervisor            // binary services; may be nil in tests
}
```

Where `Supervisor` is the type from the supervisor package. Add import:

```go
"github.com/prvious/pv/internal/supervisor"
```

And change the struct field type to `*supervisor.Supervisor`:

```go
supervisor *supervisor.Supervisor
```

Update the constructor:

```go
// NewServerManager creates a manager with the given main FrankenPHP instance
// and supervisor. The supervisor is used to manage native binary services
// (e.g. rustfs for S3) alongside the FrankenPHP secondary instances.
func NewServerManager(main *FrankenPHP, sup *supervisor.Supervisor) *ServerManager {
	return &ServerManager{
		main:        main,
		secondaries: make(map[string]*FrankenPHP),
		supervisor:  sup,
	}
}
```

- [ ] **Step 4: Add `reconcileBinaryServices`**

Add a new method in the same file:

```go
// reconcileBinaryServices brings supervisor state in line with the binary
// entries in the registry. Enabled entries are started if not yet running;
// disabled or removed entries are stopped.
// Errors from individual services are collected and logged but do not abort
// the overall reconcile.
func (m *ServerManager) reconcileBinaryServices(ctx context.Context) error {
	if m.supervisor == nil {
		return nil
	}

	reg, err := registry.Load()
	if err != nil {
		return fmt.Errorf("reconcile binary: load registry: %w", err)
	}

	needed := map[string]services.BinaryService{}
	for name, svc := range services.AllBinary() {
		entry := reg.Services[name]
		if entry == nil || entry.Kind != "binary" {
			continue
		}
		// nil Enabled means enabled (back-compat).
		if entry.Enabled != nil && !*entry.Enabled {
			continue
		}
		needed[name] = svc
	}

	// Compute supervised -> binary-service name. Supervisor uses the binary
	// name (e.g. "rustfs"), while the registry key is the service name
	// ("s3"). Derive the binary name via svc.Binary().Name to reconcile.
	neededByBinary := map[string]services.BinaryService{}
	for _, svc := range needed {
		neededByBinary[svc.Binary().Name] = svc
	}

	// Stop supervised processes no longer needed.
	for _, binName := range m.supervisor.SupervisedNames() {
		if _, ok := neededByBinary[binName]; !ok {
			if err := m.supervisor.Stop(binName, 10*time.Second); err != nil {
				fmt.Fprintf(os.Stderr, "reconcile binary: stop %s: %v\n", binName, err)
			}
		}
	}

	// Start needed processes not currently supervised.
	var startErrors []string
	for binName, svc := range neededByBinary {
		if m.supervisor.IsRunning(binName) {
			continue
		}
		proc, err := buildSupervisorProcess(svc)
		if err != nil {
			startErrors = append(startErrors, fmt.Sprintf("%s: build: %v", binName, err))
			continue
		}
		if err := m.supervisor.Start(ctx, proc); err != nil {
			startErrors = append(startErrors, fmt.Sprintf("%s: start: %v", binName, err))
			continue
		}
	}

	if len(startErrors) > 0 {
		return fmt.Errorf("binary reconcile: %d service(s) failed: %s", len(startErrors), strings.Join(startErrors, "; "))
	}
	return nil
}
```

Add `"context"` and `"time"` to imports if missing. `strings`, `os`, `fmt`, `registry`, `services` should already be imported.

- [ ] **Step 5: Extend `Reconcile()` to call the binary phase**

Locate the existing `Reconcile()` method. Just before the final `if len(startErrors) > 0 { ... }` return block, insert:

```go
// Phase 2: binary services.
if err := m.reconcileBinaryServices(context.Background()); err != nil {
	fmt.Fprintf(os.Stderr, "Reconcile: %v\n", err)
	// non-fatal — FrankenPHP-side reconcile results still returned below
}

// Phase 3: refresh daemon-status snapshot.
if err := writeDaemonStatus(m.supervisor); err != nil {
	fmt.Fprintf(os.Stderr, "Reconcile: write daemon-status: %v\n", err)
}
```

- [ ] **Step 6: Extend `Shutdown()`**

Add one line at the top of `Shutdown()`:

```go
func (m *ServerManager) Shutdown() {
	m.mu.Lock()
	defer m.mu.Unlock()

	if m.supervisor != nil {
		m.supervisor.StopAll(10 * time.Second)
	}

	// existing secondary shutdown loop
	for version, fp := range m.secondaries {
		fmt.Fprintf(os.Stderr, "Stopping FrankenPHP for PHP %s\n", version)
		fp.Stop()
		delete(m.secondaries, version)
	}
}
```

- [ ] **Step 7: Run tests to verify they pass**

```bash
gofmt -w internal/server/
go vet ./internal/server/
go test ./internal/server/ -v
```

Expected: PASS (both the new tests and any existing Reconcile tests). If existing tests fail because `NewServerManager` signature changed, update those call sites in the test to pass `nil` or a fresh `supervisor.New()`.

- [ ] **Step 8: Commit**

```bash
git add internal/server/manager.go internal/server/manager_test.go
git commit -m "Extend ServerManager.Reconcile with binary-service phase

ServerManager now owns a *supervisor.Supervisor alongside the
FrankenPHP instances. Reconcile runs the existing FrankenPHP phase
and then a new phase that diffs the binary registry against the
supervisor state, starting/stopping processes as needed.

NewServerManager signature changes to accept the supervisor.
Callers in process.go updated in the next task."
```

---

## Task 9: Wire supervisor into `server.Start()` and filter Colima recovery

**Files:**
- Modify: `internal/server/process.go`
- Modify: `internal/colima/recovery.go`

- [ ] **Step 1: Update `Start()` to create the supervisor and pass to manager**

Edit `internal/server/process.go`. Locate this block (around the current line 93):

```go
manager = NewServerManager(mainFP)
```

Replace with:

```go
sup := supervisor.New()
manager = NewServerManager(mainFP, sup)
```

Add `"github.com/prvious/pv/internal/supervisor"` to the imports.

- [ ] **Step 2: Filter Colima recovery to Docker services only**

Edit `internal/colima/recovery.go`. Locate `ServicesToRecover` (or equivalent). It currently returns every key in `reg.Services`. Update to skip binary services:

```go
func ServicesToRecover(reg *registry.Registry) []string {
	var keys []string
	for key, inst := range reg.Services {
		if inst.Kind == "binary" {
			continue
		}
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}
```

Also update the Colima-boot trigger in `internal/server/process.go`. The current code is:

```go
if colima.IsInstalled() && len(reg.ListServices()) > 0 {
	go bootColimaAndRecover(colimaCtx, settings.Defaults.VM)
}
```

Change to only fire when at least one Docker service is registered:

```go
dockerCount := 0
for _, inst := range reg.ListServices() {
	if inst.Kind != "binary" {
		dockerCount++
	}
}
if colima.IsInstalled() && dockerCount > 0 {
	go bootColimaAndRecover(colimaCtx, settings.Defaults.VM)
}
```

- [ ] **Step 3: Build and run full test suite**

```bash
gofmt -w internal/server/ internal/colima/
go vet ./...
go build ./...
go test ./...
```

Expected: PASS throughout.

- [ ] **Step 4: Commit**

```bash
git add internal/server/process.go internal/colima/recovery.go
git commit -m "Wire supervisor into daemon start and filter Colima boot

server.Start() creates a supervisor and hands it to
NewServerManager. Colima boot is now gated on the existence of
Docker-kind services so a registry with only binary services
does not trigger an unnecessary VM boot."
```

---

## Task 10: `resolveKind` dispatcher

**Files:**
- Create: `internal/commands/service/dispatch.go`
- Create: `internal/commands/service/dispatch_test.go`

- [ ] **Step 1: Write the failing tests**

Create `internal/commands/service/dispatch_test.go`:

```go
package service

import (
	"testing"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

func TestResolveKind_BinaryServiceByName(t *testing.T) {
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	kind, bin, doc, err := resolveKind(reg, "s3")
	if err != nil {
		t.Fatalf("resolveKind: %v", err)
	}
	if kind != kindBinary {
		t.Errorf("kind = %v, want kindBinary", kind)
	}
	if bin == nil {
		t.Error("binary service should be non-nil")
	}
	if doc != nil {
		t.Error("docker service should be nil")
	}
	if _, ok := bin.(*services.RustFS); !ok {
		t.Errorf("expected *RustFS, got %T", bin)
	}
}

func TestResolveKind_DockerServiceByName(t *testing.T) {
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	kind, bin, doc, err := resolveKind(reg, "mysql")
	if err != nil {
		t.Fatalf("resolveKind: %v", err)
	}
	if kind != kindDocker {
		t.Errorf("kind = %v, want kindDocker", kind)
	}
	if doc == nil {
		t.Error("docker service should be non-nil")
	}
	if bin != nil {
		t.Error("binary service should be nil")
	}
}

func TestResolveKind_Unknown_ReturnsError(t *testing.T) {
	reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
	_, _, _, err := resolveKind(reg, "bogus")
	if err == nil {
		t.Fatal("expected error for unknown service")
	}
}

func TestResolveKind_DockerEntryBlocksBinaryRegistration(t *testing.T) {
	// Pre-existing Docker "s3" entry (from older pv) should error on a
	// service:add for the now-binary "s3" — no silent auto-migration.
	reg := &registry.Registry{
		Services: map[string]*registry.ServiceInstance{
			"s3": {Kind: "", Image: "rustfs/rustfs:latest", Port: 9000},
		},
	}
	kind, _, _, err := resolveKind(reg, "s3")
	if err == nil {
		t.Fatal("expected error for pre-existing docker s3 entry")
	}
	_ = kind
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/commands/service/ -run ResolveKind -v
```

Expected: FAIL — `undefined: resolveKind`, `undefined: kindBinary`, `undefined: kindDocker`.

- [ ] **Step 3: Implement the dispatcher**

Create `internal/commands/service/dispatch.go`:

```go
package service

import (
	"fmt"

	"github.com/prvious/pv/internal/registry"
	"github.com/prvious/pv/internal/services"
)

type serviceKind int

const (
	kindUnknown serviceKind = iota
	kindDocker
	kindBinary
)

// resolveKind determines whether the named service is a binary or docker
// service, returning at most one of the concrete service values.
// If the name matches a binary service but the registry already holds a
// docker-shaped entry for that name, an error is returned: no silent
// auto-migration. The user's remedy is `pv uninstall && pv setup`.
func resolveKind(reg *registry.Registry, name string) (serviceKind, services.BinaryService, services.Service, error) {
	binSvc, binOK := services.LookupBinary(name)
	docSvc, docErr := services.Lookup(name)

	if binOK {
		// Guard against a pre-existing docker-shaped entry for what is now
		// a binary service.
		if existing, ok := reg.Services[name]; ok {
			if existing.Kind != "binary" {
				return kindUnknown, nil, nil, fmt.Errorf(
					"%s is already registered (as docker) from a previous pv version. "+
						"Run `pv uninstall && pv setup` to reset", name)
			}
		}
		return kindBinary, binSvc, nil, nil
	}
	if docErr == nil {
		return kindDocker, nil, docSvc, nil
	}
	return kindUnknown, nil, nil, docErr
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w internal/commands/service/
go vet ./internal/commands/service/
go test ./internal/commands/service/ -run ResolveKind -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/commands/service/dispatch.go internal/commands/service/dispatch_test.go
git commit -m "Add service command kind dispatcher

resolveKind is the single place that decides whether a given service
name is docker- or binary-backed. It also rejects pre-existing
docker-shaped registry entries for names that are now binary services
so migrations do not silently overwrite state."
```

---

## Task 11: `service:add` binary path

**Files:**
- Modify: `internal/commands/service/add.go`

- [ ] **Step 1: Extract the existing docker path into a helper**

Refactor `add.go` so its `RunE` closure becomes a thin dispatcher. Top of file:

```go
var addCmd = &cobra.Command{
	Use:     "service:add <service> [version]",
	GroupID: "service",
	Short:   "Add and start a service",
	Long:    "Add a backing service (mail, mysql, postgres, redis, s3). Optionally specify a version.",
	Example: `# Add MySQL with default version
pv service:add mysql

# Add S3 (RustFS binary)
pv service:add s3

# Add a specific Redis version
pv service:add redis 7`,
	Args: cobra.RangeArgs(1, 2),
	RunE: func(cmd *cobra.Command, args []string) error {
		name := args[0]

		reg, err := registry.Load()
		if err != nil {
			return fmt.Errorf("cannot load registry: %w", err)
		}

		kind, binSvc, dockerSvc, err := resolveKind(reg, name)
		if err != nil {
			return err
		}
		switch kind {
		case kindBinary:
			return addBinary(cmd.Context(), reg, binSvc)
		case kindDocker:
			version := dockerSvc.DefaultVersion()
			if len(args) > 1 {
				version = args[1]
			}
			return addDocker(cmd, reg, dockerSvc, name, version)
		}
		return fmt.Errorf("unknown service %q", name)
	},
}
```

Wrap the existing RunE body into a new function `addDocker(cmd *cobra.Command, reg *registry.Registry, svc services.Service, name, version string) error` — keep the body unchanged except for the fact that `reg` is passed in and `svcName`/`svc`/`version` are parameters.

- [ ] **Step 2: Implement `addBinary`**

Uses the real binaries API (verified): `FetchLatestVersion(client, b)` and `InstallBinary(client, b, version)`. Version state is persisted via `LoadVersions` / `Set` / `Save` so `pv update` can compare later.

Add a new function in the same file:

```go
// addBinary downloads the service's binary (if not yet present), persists its
// version, registers the service in the registry, then signals the daemon.
func addBinary(ctx context.Context, reg *registry.Registry, svc services.BinaryService) error {
	name := svc.Name()
	if _, exists := reg.Services[name]; exists {
		ui.Success(fmt.Sprintf("%s is already added", svc.DisplayName()))
		return nil
	}

	client := &http.Client{Timeout: 60 * time.Second}

	// Resolve latest upstream version.
	latest, err := binaries.FetchLatestVersion(client, svc.Binary())
	if err != nil {
		return fmt.Errorf("cannot resolve latest %s version: %w", svc.Binary().DisplayName, err)
	}

	// Download + extract into ~/.pv/internal/bin/<name>.
	if err := ui.Step(fmt.Sprintf("Downloading %s %s...", svc.Binary().DisplayName, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, svc.Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Installed %s %s", svc.Binary().DisplayName, latest), nil
	}); err != nil {
		return err
	}

	// Record version for later pv-update comparisons.
	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set(svc.Binary().Name, latest)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}

	// Register service.
	enabled := true
	inst := &registry.ServiceInstance{
		Port:        svc.Port(),
		ConsolePort: svc.ConsolePort(),
		Kind:        "binary",
		Enabled:     &enabled,
	}
	if err := reg.AddService(name, inst); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	// Regenerate Caddy configs for service consoles (*.pv.{tld}).
	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
	}

	// Signal daemon to reconcile.
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
		ui.Success(fmt.Sprintf("%s registered and running on :%d", svc.DisplayName(), svc.Port()))
	} else {
		ui.Success(fmt.Sprintf("%s registered on :%d", svc.DisplayName(), svc.Port()))
		ui.Subtle("daemon not running — service will start on next `pv start`")
	}

	printBinaryConnectionDetails(svc)
	return nil
}

// printBinaryConnectionDetails mirrors the verbose "Host / Port / web routes"
// footer that the docker path prints, scoped to the binary-service shape.
func printBinaryConnectionDetails(svc services.BinaryService) {
	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "    %s  127.0.0.1\n", ui.Muted.Render("Host"))
	fmt.Fprintf(os.Stderr, "    %s  %d\n", ui.Muted.Render("Port"), svc.Port())
	settings, _ := config.LoadSettings()
	if settings != nil {
		for _, route := range svc.WebRoutes() {
			fmt.Fprintf(os.Stderr, "    %s  https://%s.pv.%s\n",
				ui.Muted.Render(route.Subdomain), route.Subdomain, settings.Defaults.TLD)
		}
	}
	fmt.Fprintln(os.Stderr)
}
```

Add to imports: `"context"`, `"net/http"`, `"time"`, `"github.com/prvious/pv/internal/binaries"`.

- [ ] **Step 3: Build and vet**

```bash
gofmt -w internal/commands/service/
go vet ./internal/commands/service/
go build ./...
```

- [ ] **Step 4: Commit**

```bash
git add internal/commands/service/add.go
git commit -m "Implement service:add binary path

New addBinary function downloads the service's binary, registers
it with Kind=binary and Enabled=true, and signals the running
daemon to reconcile. When no daemon is running, the entry is
persisted and will be picked up by the next pv start."
```

---

## Task 12: `service:start` and `service:stop` binary paths

**Files:**
- Modify: `internal/commands/service/start.go`
- Modify: `internal/commands/service/stop.go`

- [ ] **Step 1: Implement `service:start` binary path**

Edit `internal/commands/service/start.go`. Find the `RunE` closure and wrap it with a kind dispatch. The binary branch:

```go
if kind == kindBinary {
	name := binSvc.Name()
	inst, ok := reg.Services[name]
	if !ok {
		return fmt.Errorf("%s not registered; run `pv service:add %s` first", name, name)
	}
	tru := true
	inst.Enabled = &tru
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
		ui.Success(fmt.Sprintf("%s enabled; daemon reconciled", binSvc.DisplayName()))
	} else {
		ui.Success(fmt.Sprintf("%s enabled", binSvc.DisplayName()))
		ui.Subtle("daemon not running — service will start on next `pv start`")
	}
	return nil
}
```

Preserve the existing Docker-path body for `kindDocker`.

- [ ] **Step 2: Implement `service:stop` binary path**

Edit `internal/commands/service/stop.go`. Same shape; binary branch:

```go
if kind == kindBinary {
	name := binSvc.Name()
	inst, ok := reg.Services[name]
	if !ok {
		return fmt.Errorf("%s not registered", name)
	}
	fls := false
	inst.Enabled = &fls
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}
	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
		ui.Success(fmt.Sprintf("%s disabled; daemon reconciled", binSvc.DisplayName()))
	} else {
		ui.Success(fmt.Sprintf("%s disabled", binSvc.DisplayName()))
	}
	return nil
}
```

- [ ] **Step 3: Build and test**

```bash
gofmt -w internal/commands/service/
go vet ./internal/commands/service/
go build ./...
go test ./...
```

- [ ] **Step 4: Commit**

```bash
git add internal/commands/service/start.go internal/commands/service/stop.go
git commit -m "Implement service:start and service:stop binary paths

Set Enabled on the registry entry and SignalDaemon so the daemon's
reconcile loop spawns or stops the supervised process without
restarting FrankenPHP."
```

---

## Task 13: `service:remove` and `service:destroy` binary paths

**Files:**
- Modify: `internal/commands/service/remove.go`
- Modify: `internal/commands/service/destroy.go`

- [ ] **Step 1: Implement `service:remove` binary path**

Edit `internal/commands/service/remove.go`. Binary branch:

```go
if kind == kindBinary {
	name := binSvc.Name()
	if _, ok := reg.Services[name]; !ok {
		return fmt.Errorf("%s not registered", name)
	}
	if err := reg.RemoveService(name); err != nil {
		return err
	}
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}
	// Delete the binary.
	binPath := filepath.Join(config.InternalBinDir(), binSvc.Binary().Name)
	_ = os.Remove(binPath)
	// Clear the tracked version so a future `service:add` redownloads.
	if vs, err := binaries.LoadVersions(); err == nil {
		vs.Set(binSvc.Binary().Name, "")
		_ = vs.Save()
	}

	// Regenerate Caddy configs (remove s3.pv.test / s3-api.pv.test routes).
	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not regenerate service site config: %v", err))
	}

	if server.IsRunning() {
		if err := server.SignalDaemon(); err != nil {
			ui.Subtle(fmt.Sprintf("Could not signal daemon: %v", err))
		}
	}
	ui.Success(fmt.Sprintf("%s removed (data preserved)", binSvc.DisplayName()))
	return nil
}
```

Add `"path/filepath"`, `"os"`, `"github.com/prvious/pv/internal/binaries"`, `"github.com/prvious/pv/internal/config"`, `"github.com/prvious/pv/internal/caddy"` imports if needed.

- [ ] **Step 2: Implement `service:destroy` binary path**

Edit `internal/commands/service/destroy.go`. Binary branch:

```go
if kind == kindBinary {
	name := binSvc.Name()
	_ = reg.RemoveService(name)
	if err := reg.Save(); err != nil {
		return fmt.Errorf("cannot save registry: %w", err)
	}

	binPath := filepath.Join(config.InternalBinDir(), binSvc.Binary().Name)
	_ = os.Remove(binPath)
	if vs, err := binaries.LoadVersions(); err == nil {
		vs.Set(binSvc.Binary().Name, "")
		_ = vs.Save()
	}

	dataDir := config.ServiceDataDir(name, "latest")
	if err := os.RemoveAll(dataDir); err != nil {
		return fmt.Errorf("cannot delete data: %w", err)
	}

	if err := caddy.GenerateServiceSiteConfigs(reg); err != nil {
		ui.Subtle(fmt.Sprintf("Could not regenerate service site config: %v", err))
	}
	if server.IsRunning() {
		_ = server.SignalDaemon()
	}
	ui.Success(fmt.Sprintf("%s destroyed (binary + data gone)", binSvc.DisplayName()))
	return nil
}
```

- [ ] **Step 3: Build and test**

```bash
gofmt -w internal/commands/service/
go vet ./internal/commands/service/
go build ./...
go test ./...
```

- [ ] **Step 4: Commit**

```bash
git add internal/commands/service/remove.go internal/commands/service/destroy.go
git commit -m "Implement service:remove and service:destroy binary paths

remove unregisters and deletes the binary but keeps data. destroy
also removes config.ServiceDataDir. Both signal the daemon so the
supervised process is stopped."
```

---

## Task 14: `service:status` and `service:list` observation commands

**Files:**
- Modify: `internal/commands/service/status.go`
- Modify: `internal/commands/service/list.go`

- [ ] **Step 1: Update `service:status` to read daemon-status.json**

Edit `internal/commands/service/status.go`. Add a binary-kind branch that reads `~/.pv/daemon-status.json` via `server.ReadDaemonStatus()`:

```go
if kind == kindBinary {
	name := binSvc.Name()
	inst, ok := reg.Services[name]
	enabled := true
	registered := ok
	if ok && inst.Enabled != nil {
		enabled = *inst.Enabled
	}

	running := false
	pid := 0
	if snap, err := server.ReadDaemonStatus(); err == nil {
		if st, ok := snap.Supervised[binSvc.Binary().Name]; ok {
			running = st.Running
			pid = st.PID
		}
	}

	fmt.Fprintln(os.Stderr)
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Service"), binSvc.DisplayName())
	fmt.Fprintf(os.Stderr, "  %s  %s\n", ui.Muted.Render("Kind"), "binary")
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Registered"), registered)
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Enabled"), enabled)
	fmt.Fprintf(os.Stderr, "  %s  %v\n", ui.Muted.Render("Running"), running)
	if pid > 0 {
		fmt.Fprintf(os.Stderr, "  %s  %d\n", ui.Muted.Render("PID"), pid)
	}
	fmt.Fprintln(os.Stderr)
	return nil
}
```

- [ ] **Step 2: Update `service:list` to show both kinds**

Edit `internal/commands/service/list.go`. After assembling the Docker rows, iterate binary entries too:

```go
// Append binary entries after docker ones.
snap, _ := server.ReadDaemonStatus()
for name, inst := range reg.Services {
	if inst.Kind != "binary" {
		continue
	}
	svc, ok := services.LookupBinary(name)
	if !ok {
		continue
	}
	enabled := true
	if inst.Enabled != nil {
		enabled = *inst.Enabled
	}
	running := false
	if snap != nil {
		if st, ok := snap.Supervised[svc.Binary().Name]; ok {
			running = st.Running
		}
	}
	status := "stopped"
	if running {
		status = "running"
	} else if !enabled {
		status = "disabled"
	}
	rows = append(rows, []string{
		name,
		"binary",
		fmt.Sprintf(":%d", inst.Port),
		status,
	})
}
```

(The exact table-assembly code depends on the current `list.go` structure; adapt column order / headers to match the existing docker output.)

- [ ] **Step 3: Build and test**

```bash
gofmt -w internal/commands/service/
go vet ./internal/commands/service/
go build ./...
go test ./...
```

- [ ] **Step 4: Commit**

```bash
git add internal/commands/service/status.go internal/commands/service/list.go
git commit -m "Read daemon-status.json for binary service observability

service:status shows Kind/Registered/Enabled/Running/PID for
binary services. service:list merges docker and binary rows so
users see a unified view."
```

---

## Task 15: `service:logs` binary path

**Files:**
- Modify: `internal/commands/service/logs.go`

- [ ] **Step 1: Implement the binary branch**

Edit `internal/commands/service/logs.go`. Binary branch:

```go
if kind == kindBinary {
	logPath := filepath.Join(config.PvDir(), "logs", binSvc.Binary().Name+".log")
	f, err := os.Open(logPath)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("no log file yet (%s). Has the service run?", logPath)
		}
		return err
	}
	defer f.Close()
	// Simple 'tail -f' style: dump existing content then follow appends.
	if _, err := io.Copy(os.Stdout, f); err != nil {
		return err
	}
	// Follow mode (like tail -f). Poll every 250ms for new data; exit on Ctrl-C.
	for {
		select {
		case <-cmd.Context().Done():
			return nil
		case <-time.After(250 * time.Millisecond):
		}
		if _, err := io.Copy(os.Stdout, f); err != nil {
			if err == io.EOF {
				continue
			}
			return err
		}
	}
}
```

Add imports: `"io"`, `"time"`, `"path/filepath"`, `"github.com/prvious/pv/internal/config"`.

- [ ] **Step 2: Build and test**

```bash
gofmt -w internal/commands/service/
go vet ./internal/commands/service/
go build ./...
```

- [ ] **Step 3: Commit**

```bash
git add internal/commands/service/logs.go
git commit -m "Implement service:logs binary path via tail-f of log file

Binary services write stdout+stderr to ~/.pv/logs/<binary>.log via
the supervisor. service:logs dumps existing content and follows
appends, exiting on context cancellation."
```

---

## Task 16: `pv update` hook for binary services

**Files:**
- Modify: `cmd/update.go`

- [ ] **Step 1: Append a binary-services loop after the existing tool loop**

Uses the real version-tracking API: `LoadVersions()` returns a `VersionState`, `vs.Get(name)` returns the installed version (or empty), `NeedsUpdate` handles `v1.x.x` vs `1.x.x` normalization, and `vs.Set` + `vs.Save()` persist. No sidecar files.

Edit `cmd/update.go`. Near the end of the `RunE`, after all tool updates finish but before the final footer, add:

```go
// Update binary-service binaries.
reg, _ := registry.Load()
vs, _ := binaries.LoadVersions()
client := &http.Client{Timeout: 60 * time.Second}

var binaryUpdated []string
for name, svc := range services.AllBinary() {
	if _, registered := reg.Services[name]; !registered {
		continue
	}
	latest, err := binaries.FetchLatestVersion(client, svc.Binary())
	if err != nil {
		ui.Subtle(fmt.Sprintf("Skipping %s: %v", svc.DisplayName(), err))
		continue
	}
	if !binaries.NeedsUpdate(vs, svc.Binary(), latest) {
		continue
	}
	current := vs.Get(svc.Binary().Name)
	if err := ui.Step(fmt.Sprintf("Updating %s %s -> %s", svc.Binary().DisplayName, current, latest), func() (string, error) {
		if err := binaries.InstallBinary(client, svc.Binary(), latest); err != nil {
			return "", err
		}
		return fmt.Sprintf("Updated %s to %s", svc.Binary().DisplayName, latest), nil
	}); err != nil {
		ui.Subtle(fmt.Sprintf("Could not update %s: %v", svc.DisplayName(), err))
		continue
	}
	vs.Set(svc.Binary().Name, latest)
	binaryUpdated = append(binaryUpdated, name)
}
if len(binaryUpdated) > 0 {
	if err := vs.Save(); err != nil {
		ui.Subtle(fmt.Sprintf("Could not save versions state: %v", err))
	}
	if server.IsRunning() {
		ui.Subtle(fmt.Sprintf("Updated binaries: %s. Run `pv service:stop %s && pv service:start %s` (or `pv restart`) to load them.",
			strings.Join(binaryUpdated, ", "), binaryUpdated[0], binaryUpdated[0]))
	}
}
```

Add imports: `"github.com/prvious/pv/internal/registry"`, `"github.com/prvious/pv/internal/services"`, `"github.com/prvious/pv/internal/binaries"`, `"github.com/prvious/pv/internal/server"`, `"net/http"`, `"strings"`, `"time"` (as needed).

- [ ] **Step 2: Build**

```bash
gofmt -w cmd/
go vet ./...
go build ./...
```

- [ ] **Step 3: Commit**

```bash
git add cmd/update.go
git commit -m "Refresh binary-service binaries in pv update

After the tool loop, iterate registered binary services and
compare installed vs latest upstream version. Newer binaries are
downloaded; user is advised to cycle the service (or pv restart)
to load them since the running process keeps the old binary via
its open file descriptor."
```

---

## Task 17: E2E test + CI integration

**Files:**
- Create: `scripts/e2e/s3-binary.sh`
- Modify: `.github/workflows/e2e.yml`

- [ ] **Step 1: Write the E2E script**

Create `scripts/e2e/s3-binary.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./helpers.sh
. "$SCRIPT_DIR/helpers.sh"

phase "S3 binary service (RustFS) lifecycle"

pv start >/dev/null &
START_PID=$!
sleep 3

trap 'kill $START_PID 2>/dev/null || true; pv stop >/dev/null 2>&1 || true' EXIT

step "service:add s3"
pv service:add s3

step "binary exists"
test -x "$HOME/.pv/internal/bin/rustfs"

step "daemon-status lists rustfs"
test -f "$HOME/.pv/daemon-status.json"
grep -q '"rustfs"' "$HOME/.pv/daemon-status.json"

step "port 9000 is reachable"
for i in $(seq 1 20); do
    if nc -z 127.0.0.1 9000 2>/dev/null; then break; fi
    sleep 1
done
nc -z 127.0.0.1 9000

step "service:stop s3"
pv service:stop s3
sleep 2
if nc -z 127.0.0.1 9000 2>/dev/null; then
    echo "FAIL: port 9000 still answering after service:stop"
    exit 1
fi

step "service:start s3"
pv service:start s3
for i in $(seq 1 20); do
    if nc -z 127.0.0.1 9000 2>/dev/null; then break; fi
    sleep 1
done
nc -z 127.0.0.1 9000

step "service:destroy s3"
pv service:destroy s3
test ! -f "$HOME/.pv/internal/bin/rustfs"
test ! -d "$HOME/.pv/services/s3/latest/data"

step "pv stop"
pv stop || true
trap - EXIT

pass "S3 binary service lifecycle OK"
```

Make it executable:

```bash
chmod +x scripts/e2e/s3-binary.sh
```

- [ ] **Step 2: Wire into the CI workflow**

Edit `.github/workflows/e2e.yml`. Add a step after the existing service-related phase:

```yaml
      - name: E2E — S3 binary service lifecycle
        run: ./scripts/e2e/s3-binary.sh
```

- [ ] **Step 3: Run locally on macOS before pushing**

```bash
go build -o pv .
./scripts/e2e/s3-binary.sh
```

Expected: all steps PASS; on failure, inspect `~/.pv/logs/rustfs.log` for RustFS stderr output.

- [ ] **Step 4: Commit**

```bash
git add scripts/e2e/s3-binary.sh .github/workflows/e2e.yml
git commit -m "Add E2E phase for S3 binary service lifecycle

Exercises service:add, service:stop, service:start, service:destroy
against a real RustFS download. Verifies the binary is written,
the daemon-status file advertises the supervised process, and
port 9000 is reachable / silent at the expected moments."
```

---

## Parallelization Guide

Most of this plan is linear. Once Task 6 (supervisor) and Task 8 (manager) are committed, these can run in parallel:

| Agent | Tasks | Files touched |
|-------|-------|---------------|
| A | Task 11 (service:add) | add.go |
| B | Task 12 (start/stop) | start.go, stop.go |
| C | Task 13 (remove/destroy) | remove.go, destroy.go |
| D | Task 14 (status/list) | status.go, list.go |
| E | Task 15 (logs) | logs.go |

Tasks 16 (pv update) and 17 (E2E) must run after 11-15 merge.

Tasks 1 through 10 are strictly linear — each builds on the previous.
