# Mailpit RustFS Versioned Archives Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert Mailpit and RustFS from `latest` singleton internal binaries to archived, version-line services installed under top-level `~/.pv/{service}/{version}` roots, and make `pv.yml` omitted service versions resolve to pv defaults.

**Architecture:** This plan normalizes the service model first; shared lifecycle helper extraction is intentionally deferred to a follow-up plan. Mailpit and RustFS keep explicit packages and command groups, but their artifact, path, state, registry, and supervisor identity now match the version-line model used by Postgres/MySQL/Redis.

**Tech Stack:** Go 1.26 target, Cobra, GitHub Actions, `internal/binaries`, `internal/config`, `internal/{mailpit,rustfs}`, `internal/automation`, `internal/registry`, `internal/server`, `internal/caddy`.

---

## File Structure

- Modify `.github/workflows/build-artifacts.yml` to upload `mailpit-*.tar.gz` and `rustfs-*.tar.gz` archives containing `bin/<binary>` plus a `VERSION` file with the upstream release tag.
- Modify `internal/config/paths.go` and `internal/config/paths_test.go` to add top-level Mailpit/RustFS binary roots and normalized data/log helpers.
- Modify `internal/binaries/mailpit.go`, `internal/binaries/rustfs.go`, `internal/binaries/manager.go`, and tests to resolve pv artifact URLs instead of upstream release URLs for managed service installs.
- Modify `internal/mailpit/*` and `internal/rustfs/*` to use stable defaults (`1`, `1.0.0-beta`), versioned binary roots, normalized data dirs, archive extraction, version recording, and updated tests.
- Modify `internal/automation/steps/apply_pvyml_services.go` and tests so any declared service block with an omitted `version` resolves to that service package's default version line.
- Modify `internal/config/pvyml.go` and `internal/config/pvyml_test.go` comments/examples to document explicit generated versions and omitted-version defaulting.
- Modify `internal/commands/{mailpit,rustfs}/{start,stop,restart,install,update,uninstall}.go`, `cmd/update.go`, `cmd/uninstall.go`, `cmd/setup.go`, `internal/caddy/caddy.go`, and `internal/server/manager.go` only where tests expose hard-coded `latest` assumptions or daemon signaling gaps.

---

### Task 1: Normalize Mailpit/RustFS Artifact Uploads

**Files:**
- Modify: `.github/workflows/build-artifacts.yml:440-557`

- [ ] **Step 1: Update Mailpit repack layout**

Replace the Mailpit `Repack` step with this archive layout:

```yaml
      - name: Repack
        run: |
          set -euo pipefail
          mkdir -p package/bin
          cp staging/mailpit package/bin/mailpit
          printf "%s\n" "${{ steps.resolve.outputs.tag }}" > package/VERSION
          tar -czf "mailpit-mac-arm64-${MAILPIT_VERSION_MAJOR}.tar.gz" -C package bin VERSION
```

- [ ] **Step 2: Replace RustFS naked-binary upload with an archive**

Replace the RustFS `Rename binary` step and upload path with this:

```yaml
      - name: Repack
        run: |
          set -euo pipefail
          mkdir -p package/bin
          cp staging/rustfs package/bin/rustfs
          printf "%s\n" "${RUSTFS_TAG}" > package/VERSION
          tar -czf "rustfs-mac-arm64-${RUSTFS_VERSION_MAJOR}.tar.gz" -C package bin VERSION

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: rustfs-mac-arm64-${{ env.RUSTFS_VERSION_MAJOR }}
          path: rustfs-mac-arm64-${{ env.RUSTFS_VERSION_MAJOR }}.tar.gz
          compression-level: 0
```

- [ ] **Step 3: Verify workflow references**

Run: `git diff -- .github/workflows/build-artifacts.yml`

Expected: Mailpit and RustFS both upload `.tar.gz` paths; no RustFS upload path points at a naked `rustfs-mac-arm64-*` executable.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "build: archive rustfs and mailpit artifacts"
```

---

### Task 2: Add Canonical Config Paths

**Files:**
- Modify: `internal/config/paths.go`
- Modify: `internal/config/paths_test.go`

- [ ] **Step 1: Add failing path tests**

Append these tests to `internal/config/paths_test.go`:

```go
func TestMailpitPaths(t *testing.T) {
	t.Setenv("HOME", "/home/test")

	if got, want := MailpitDir(), "/home/test/.pv/mailpit"; got != want {
		t.Errorf("MailpitDir = %q, want %q", got, want)
	}
	if got, want := MailpitVersionDir("1"), "/home/test/.pv/mailpit/1"; got != want {
		t.Errorf("MailpitVersionDir = %q, want %q", got, want)
	}
	if got, want := MailpitBinDir("1"), "/home/test/.pv/mailpit/1/bin"; got != want {
		t.Errorf("MailpitBinDir = %q, want %q", got, want)
	}
	if got, want := MailpitDataDir("1"), "/home/test/.pv/data/mailpit/1"; got != want {
		t.Errorf("MailpitDataDir = %q, want %q", got, want)
	}
	if got, want := MailpitLogPath("1"), "/home/test/.pv/logs/mailpit-1.log"; got != want {
		t.Errorf("MailpitLogPath = %q, want %q", got, want)
	}
}

func TestRustfsPaths(t *testing.T) {
	t.Setenv("HOME", "/home/test")

	if got, want := RustfsDir(), "/home/test/.pv/rustfs"; got != want {
		t.Errorf("RustfsDir = %q, want %q", got, want)
	}
	if got, want := RustfsVersionDir("1.0.0-beta"), "/home/test/.pv/rustfs/1.0.0-beta"; got != want {
		t.Errorf("RustfsVersionDir = %q, want %q", got, want)
	}
	if got, want := RustfsBinDir("1.0.0-beta"), "/home/test/.pv/rustfs/1.0.0-beta/bin"; got != want {
		t.Errorf("RustfsBinDir = %q, want %q", got, want)
	}
	if got, want := RustfsDataDir("1.0.0-beta"), "/home/test/.pv/data/rustfs/1.0.0-beta"; got != want {
		t.Errorf("RustfsDataDir = %q, want %q", got, want)
	}
	if got, want := RustfsLogPath("1.0.0-beta"), "/home/test/.pv/logs/rustfs-1.0.0-beta.log"; got != want {
		t.Errorf("RustfsLogPath = %q, want %q", got, want)
	}
}
```

- [ ] **Step 2: Run the focused path tests and verify failure**

Run: `go test ./internal/config -run 'Test(Mailpit|Rustfs)Paths'`

Expected: FAIL because the new config functions do not exist.

- [ ] **Step 3: Add path helpers**

Add this code to `internal/config/paths.go` near the Redis path helpers:

```go
func MailpitDir() string {
	return filepath.Join(PvDir(), "mailpit")
}

func MailpitVersionDir(version string) string {
	return filepath.Join(MailpitDir(), version)
}

func MailpitBinDir(version string) string {
	return filepath.Join(MailpitVersionDir(version), "bin")
}

func MailpitDataDir(version string) string {
	return filepath.Join(DataDir(), "mailpit", version)
}

func MailpitLogPath(version string) string {
	return filepath.Join(LogsDir(), "mailpit-"+version+".log")
}

func RustfsDir() string {
	return filepath.Join(PvDir(), "rustfs")
}

func RustfsVersionDir(version string) string {
	return filepath.Join(RustfsDir(), version)
}

func RustfsBinDir(version string) string {
	return filepath.Join(RustfsVersionDir(version), "bin")
}

func RustfsDataDir(version string) string {
	return filepath.Join(DataDir(), "rustfs", version)
}

func RustfsLogPath(version string) string {
	return filepath.Join(LogsDir(), "rustfs-"+version+".log")
}
```

Update `EnsureDirs` to include `MailpitDir()` and `RustfsDir()`.

- [ ] **Step 4: Run focused path tests and verify pass**

Run: `go test ./internal/config -run 'Test(Mailpit|Rustfs)Paths|TestEnsureDirs'`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/config/paths.go internal/config/paths_test.go
git commit -m "refactor(config): add versioned service paths"
```

---

### Task 3: Resolve Mailpit/RustFS Artifact URLs From pv Artifacts

**Files:**
- Modify: `internal/binaries/mailpit.go`
- Modify: `internal/binaries/rustfs.go`
- Modify: `internal/binaries/manager.go`
- Modify: `internal/binaries/mailpit_test.go`
- Modify: `internal/binaries/rustfs_test.go`

- [ ] **Step 1: Write Mailpit URL tests**

Replace or add tests in `internal/binaries/mailpit_test.go`:

```go
func TestMailpitURL(t *testing.T) {
	t.Setenv("PV_MAILPIT_URL_OVERRIDE", "")
	url, err := MailpitURL("1")
	if err != nil {
		t.Fatalf("MailpitURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/mailpit-mac-arm64-1.tar.gz"
	if url != want {
		t.Fatalf("MailpitURL = %q, want %q", url, want)
	}
}

func TestMailpitURL_Override(t *testing.T) {
	t.Setenv("PV_MAILPIT_URL_OVERRIDE", "http://example.test/mailpit.tar.gz")
	url, err := MailpitURL("1")
	if err != nil {
		t.Fatalf("MailpitURL override: %v", err)
	}
	if url != "http://example.test/mailpit.tar.gz" {
		t.Fatalf("MailpitURL override = %q", url)
	}
}
```

- [ ] **Step 2: Write RustFS URL tests**

Replace or add tests in `internal/binaries/rustfs_test.go`:

```go
func TestRustfsURL(t *testing.T) {
	t.Setenv("PV_RUSTFS_URL_OVERRIDE", "")
	url, err := RustfsURL("1.0.0-beta")
	if err != nil {
		t.Fatalf("RustfsURL: %v", err)
	}
	want := "https://github.com/prvious/pv/releases/download/artifacts/rustfs-mac-arm64-1.0.0-beta.tar.gz"
	if url != want {
		t.Fatalf("RustfsURL = %q, want %q", url, want)
	}
}

func TestRustfsURL_Override(t *testing.T) {
	t.Setenv("PV_RUSTFS_URL_OVERRIDE", "http://example.test/rustfs.tar.gz")
	url, err := RustfsURL("1.0.0-beta")
	if err != nil {
		t.Fatalf("RustfsURL override: %v", err)
	}
	if url != "http://example.test/rustfs.tar.gz" {
		t.Fatalf("RustfsURL override = %q", url)
	}
}
```

- [ ] **Step 3: Run URL tests and verify failure**

Run: `go test ./internal/binaries -run 'Test(Mailpit|Rustfs)URL'`

Expected: FAIL because `MailpitURL` and `RustfsURL` do not exist or still point at upstream URLs.

- [ ] **Step 4: Implement artifact URL functions**

Use this shape in `internal/binaries/mailpit.go`:

```go
var mailpitPlatformNames = map[string]map[string]string{
	"darwin": {"arm64": "mac-arm64"},
}

func MailpitURL(version string) (string, error) {
	if override := os.Getenv("PV_MAILPIT_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	archMap, ok := mailpitPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for Mailpit: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for Mailpit: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/mailpit-%s-%s.tar.gz", platform, version), nil
}
```

Use this shape in `internal/binaries/rustfs.go`:

```go
var rustfsPlatformNames = map[string]map[string]string{
	"darwin": {"arm64": "mac-arm64"},
}

func RustfsURL(version string) (string, error) {
	if override := os.Getenv("PV_RUSTFS_URL_OVERRIDE"); override != "" {
		return override, nil
	}
	archMap, ok := rustfsPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for RustFS: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for RustFS: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("https://github.com/prvious/pv/releases/download/artifacts/rustfs-%s-%s.tar.gz", platform, version), nil
}
```

Update `DownloadURL` in `internal/binaries/manager.go` so `mailpit` calls `MailpitURL(version)` and `rustfs` calls `RustfsURL(version)`. Keep `InstallBinaryProgress` cases for now so existing callers compile; later tasks stop using those cases.

- [ ] **Step 5: Run URL tests and verify pass**

Run: `go test ./internal/binaries -run 'Test(Mailpit|Rustfs)URL'`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add internal/binaries/mailpit.go internal/binaries/rustfs.go internal/binaries/manager.go internal/binaries/mailpit_test.go internal/binaries/rustfs_test.go
git commit -m "refactor(binaries): resolve service artifact URLs"
```

---

### Task 4: Convert Mailpit To Version-Line Paths And Archive Install

**Files:**
- Modify: `internal/mailpit/version.go`
- Modify: `internal/mailpit/installed.go`
- Modify: `internal/mailpit/install.go`
- Modify: `internal/mailpit/update.go`
- Modify: `internal/mailpit/uninstall.go`
- Modify: `internal/mailpit/service.go`
- Modify: `internal/mailpit/lifecycle_test.go`
- Modify: `internal/mailpit/service_test.go`
- Modify: `internal/mailpit/templatevars_test.go`

- [ ] **Step 1: Update Mailpit lifecycle tests first**

Change Mailpit test expectations from `latest` and `InternalBinDir` to version `1` and top-level paths. The key assertions should be:

```go
func TestValidateVersion_RejectsLatest(t *testing.T) {
	if err := ValidateVersion("latest"); err == nil {
		t.Fatal("expected latest mailpit version to fail")
	}
}

func TestBuildSupervisorProcess_NameAndPaths(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess(DefaultVersion())
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}

	if proc.Name != "mailpit-1" {
		t.Errorf("proc.Name = %q, want %q", proc.Name, "mailpit-1")
	}
	if !strings.HasSuffix(proc.Binary, "/.pv/mailpit/1/bin/mailpit") {
		t.Errorf("proc.Binary = %q, want versioned mailpit binary path", proc.Binary)
	}
	if !strings.HasSuffix(proc.LogFile, "/.pv/logs/mailpit-1.log") {
		t.Errorf("proc.LogFile = %q, want versioned mailpit log path", proc.LogFile)
	}
	if _, err := os.Stat(config.MailpitDataDir("1")); err != nil {
		t.Errorf("mailpit data dir was not created: %v", err)
	}
}
```

Update fake installed binaries in tests to write `filepath.Join(config.MailpitBinDir(DefaultVersion()), "mailpit")`.

- [ ] **Step 2: Run Mailpit tests and verify failure**

Run: `go test ./internal/mailpit`

Expected: FAIL because production code still uses `latest`, `config.InternalBinDir()`, and `config.ServiceDataDir("mail", version)`.

- [ ] **Step 3: Replace Mailpit version validation**

Replace `internal/mailpit/version.go` with:

```go
package mailpit

import "fmt"

const defaultVersion = "1"

func DefaultVersion() string { return defaultVersion }

func ResolveVersion(version string) (string, error) {
	if version == "" {
		return DefaultVersion(), nil
	}
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return version, nil
}

func ValidateVersion(version string) error {
	if version != DefaultVersion() {
		return fmt.Errorf("mailpit: unsupported version %q (only %q is currently supported)", version, DefaultVersion())
	}
	return nil
}
```

- [ ] **Step 4: Replace Mailpit path helpers**

Update `internal/mailpit/installed.go` to use config helpers:

```go
func BinaryPath(version string) (string, error) {
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return filepath.Join(config.MailpitBinDir(version), Binary().Name), nil
}

func LogPath(version string) (string, error) {
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return config.MailpitLogPath(version), nil
}

func IsInstalled(version string) bool {
	path, err := BinaryPath(version)
	if err != nil {
		return false
	}
	st, err := os.Stat(path)
	return err == nil && !st.IsDir()
}
```

Keep `InstalledVersions` returning `[DefaultVersion()]` only when the versioned binary exists.

- [ ] **Step 5: Implement archive extraction for Mailpit install/update**

In `internal/mailpit/install.go`, replace `FetchLatestVersion` and `InstallBinaryProgress` usage with direct archive download/extract:

```go
func installArchive(client *http.Client, version string, progress binaries.ProgressFunc) error {
	url, err := binaries.MailpitURL(version)
	if err != nil {
		return err
	}
	versionDir := config.MailpitVersionDir(version)
	stagingDir := versionDir + ".new"
	os.RemoveAll(stagingDir)
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}
	archive := filepath.Join(config.PvDir(), "mailpit-"+version+".tar.gz")
	if err := binaries.DownloadProgress(client, url, archive, progress); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("download: %w", err)
	}
	if err := binaries.ExtractTarGzAll(archive, stagingDir); err != nil {
		os.RemoveAll(stagingDir)
		os.Remove(archive)
		return fmt.Errorf("extract: %w", err)
	}
	os.Remove(archive)
	os.RemoveAll(versionDir)
	if err := os.Rename(stagingDir, versionDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename staging: %w", err)
	}
	binPath, err := BinaryPath(version)
	if err != nil {
		return err
	}
	return binaries.MakeExecutable(binPath)
}
```

Add this version recorder in `internal/mailpit/install.go`:

```go
func recordInstalledVersion(version string) error {
	recorded := version
	data, err := os.ReadFile(filepath.Join(config.MailpitVersionDir(version), "VERSION"))
	if err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("read mailpit artifact version: %w", err)
	}
	if err == nil {
		if trimmed := strings.TrimSpace(string(data)); trimmed != "" {
			recorded = trimmed
		}
	}
	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set("mailpit-"+version, recorded)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}
	return nil
}
```

Add `strings` to the imports.

Update `InstallProgress` to call `installArchive`, create `config.MailpitDataDir(version)`, call `recordInstalledVersion`, then `SetWanted(version, WantedRunning)`.

Update `UpdateProgress` to call the same archive installation path after confirming `IsInstalled(version)`.

- [ ] **Step 6: Update Mailpit process/data/uninstall paths**

In `internal/mailpit/service.go`, use `config.MailpitDataDir(version)` instead of `config.ServiceDataDir(serviceKey, version)`.

In `internal/mailpit/uninstall.go`, remove `config.MailpitVersionDir(version)` instead of removing only the binary file, remove `config.MailpitDataDir(version)` when `force` is true, and delete `versions.json` key `mailpit-` plus version.

- [ ] **Step 7: Run Mailpit tests and verify pass**

Run: `go test ./internal/mailpit`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add internal/mailpit internal/config internal/binaries
git commit -m "refactor(mailpit): install versioned archive artifacts"
```

---

### Task 5: Convert RustFS To Version-Line Paths And Archive Install

**Files:**
- Modify: `internal/rustfs/version.go`
- Modify: `internal/rustfs/installed.go`
- Modify: `internal/rustfs/install.go`
- Modify: `internal/rustfs/update.go`
- Modify: `internal/rustfs/uninstall.go`
- Modify: `internal/rustfs/service.go`
- Modify: `internal/rustfs/lifecycle_test.go`
- Modify: `internal/rustfs/service_test.go`
- Modify: `internal/rustfs/templatevars_test.go`

- [ ] **Step 1: Update RustFS lifecycle tests first**

Change RustFS test expectations from `latest` and `InternalBinDir` to version `1.0.0-beta` and top-level paths. The key assertions should be:

```go
func TestValidateVersion_RejectsLatest(t *testing.T) {
	if err := ValidateVersion("latest"); err == nil {
		t.Fatal("expected latest rustfs version to fail")
	}
}

func TestBuildSupervisorProcess_NameAndPaths(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	proc, err := BuildSupervisorProcess(DefaultVersion())
	if err != nil {
		t.Fatalf("BuildSupervisorProcess: %v", err)
	}

	if proc.Name != "rustfs-1.0.0-beta" {
		t.Errorf("proc.Name = %q, want %q", proc.Name, "rustfs-1.0.0-beta")
	}
	if !strings.HasSuffix(proc.Binary, "/.pv/rustfs/1.0.0-beta/bin/rustfs") {
		t.Errorf("proc.Binary = %q, want versioned rustfs binary path", proc.Binary)
	}
	if !strings.HasSuffix(proc.LogFile, "/.pv/logs/rustfs-1.0.0-beta.log") {
		t.Errorf("proc.LogFile = %q, want versioned rustfs log path", proc.LogFile)
	}
	if _, err := os.Stat(config.RustfsDataDir("1.0.0-beta")); err != nil {
		t.Errorf("rustfs data dir was not created: %v", err)
	}
}
```

Update fake installed binaries in tests to write `filepath.Join(config.RustfsBinDir(DefaultVersion()), "rustfs")`.

- [ ] **Step 2: Run RustFS tests and verify failure**

Run: `go test ./internal/rustfs`

Expected: FAIL because production code still uses `latest`, `config.InternalBinDir()`, and `config.ServiceDataDir("s3", version)`.

- [ ] **Step 3: Replace RustFS version validation**

Replace `internal/rustfs/version.go` with:

```go
package rustfs

import "fmt"

const defaultVersion = "1.0.0-beta"

func DefaultVersion() string { return defaultVersion }

func ResolveVersion(version string) (string, error) {
	if version == "" {
		return DefaultVersion(), nil
	}
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return version, nil
}

func ValidateVersion(version string) error {
	if version != DefaultVersion() {
		return fmt.Errorf("rustfs: unsupported version %q (only %q is currently supported)", version, DefaultVersion())
	}
	return nil
}
```

- [ ] **Step 4: Replace RustFS path helpers**

Update `internal/rustfs/installed.go` to use config helpers:

```go
func BinaryPath(version string) (string, error) {
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return filepath.Join(config.RustfsBinDir(version), Binary().Name), nil
}

func LogPath(version string) (string, error) {
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return config.RustfsLogPath(version), nil
}
```

- [ ] **Step 5: Implement archive extraction for RustFS install/update**

In `internal/rustfs/install.go`, replace `FetchLatestVersion` and `InstallBinaryProgress` usage with direct archive download/extract:

```go
func installArchive(client *http.Client, version string, progress binaries.ProgressFunc) error {
	url, err := binaries.RustfsURL(version)
	if err != nil {
		return err
	}
	versionDir := config.RustfsVersionDir(version)
	stagingDir := versionDir + ".new"
	os.RemoveAll(stagingDir)
	if err := os.MkdirAll(stagingDir, 0o755); err != nil {
		return fmt.Errorf("create staging: %w", err)
	}
	archive := filepath.Join(config.PvDir(), "rustfs-"+version+".tar.gz")
	if err := binaries.DownloadProgress(client, url, archive, progress); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("download: %w", err)
	}
	if err := binaries.ExtractTarGzAll(archive, stagingDir); err != nil {
		os.RemoveAll(stagingDir)
		os.Remove(archive)
		return fmt.Errorf("extract: %w", err)
	}
	os.Remove(archive)
	os.RemoveAll(versionDir)
	if err := os.Rename(stagingDir, versionDir); err != nil {
		os.RemoveAll(stagingDir)
		return fmt.Errorf("rename staging: %w", err)
	}
	binPath, err := BinaryPath(version)
	if err != nil {
		return err
	}
	return binaries.MakeExecutable(binPath)
}
```

Add this version recorder in `internal/rustfs/install.go`:

```go
func recordInstalledVersion(version string) error {
	recorded := version
	data, err := os.ReadFile(filepath.Join(config.RustfsVersionDir(version), "VERSION"))
	if err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("read rustfs artifact version: %w", err)
	}
	if err == nil {
		if trimmed := strings.TrimSpace(string(data)); trimmed != "" {
			recorded = trimmed
		}
	}
	vs, err := binaries.LoadVersions()
	if err != nil {
		return fmt.Errorf("cannot load versions state: %w", err)
	}
	vs.Set("rustfs-"+version, recorded)
	if err := vs.Save(); err != nil {
		return fmt.Errorf("cannot save versions state: %w", err)
	}
	return nil
}
```

Add `strings` to the imports.

Update `InstallProgress` to call `installArchive`, create `config.RustfsDataDir(version)`, call `recordInstalledVersion`, then `SetWanted(version, WantedRunning)`.

Update `UpdateProgress` to call the same archive installation path after confirming `IsInstalled(version)`.

- [ ] **Step 6: Update RustFS process/data/uninstall paths**

In `internal/rustfs/service.go`, use `config.RustfsDataDir(version)` instead of `config.ServiceDataDir(serviceKey, version)`.

In `internal/rustfs/uninstall.go`, remove `config.RustfsVersionDir(version)` instead of removing only the binary file, remove `config.RustfsDataDir(version)` when `force` is true, and delete `versions.json` key `rustfs-` plus version.

- [ ] **Step 7: Run RustFS tests and verify pass**

Run: `go test ./internal/rustfs`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add internal/rustfs internal/config internal/binaries
git commit -m "refactor(rustfs): install versioned archive artifacts"
```

---

### Task 6: Default Service Versions In pv.yml Binding

**Files:**
- Modify: `internal/postgres/version.go`
- Modify: `internal/mysql/version.go`
- Modify: `internal/commands/postgres/install.go`
- Modify: `internal/commands/mysql/install.go`
- Modify: `internal/config/pvyml.go`
- Modify: `internal/automation/steps/apply_pvyml_services.go`
- Modify: `internal/automation/steps/apply_pvyml_services_test.go`

- [ ] **Step 1: Add failing pv.yml default tests**

In `internal/automation/steps/apply_pvyml_services_test.go`, change the existing Postgres/MySQL missing-version error tests into default-binding tests. Add these assertions:

```go
func TestApplyPvYmlServices_BindsPostgresDefaultVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stagePostgresBinary(t, postgres.DefaultVersion())

	reg := &registry.Registry{Projects: []registry.Project{{Name: "app", Path: t.TempDir()}}}
	ctx := &automation.Context{
		ProjectName: "app",
		Registry:    reg,
		ProjectConfig: &config.ProjectConfig{
			Postgresql: &config.ServiceConfig{},
		},
	}

	_, err := (&ApplyPvYmlServicesStep{}).Run(ctx)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if got := reg.Projects[0].Services.Postgres; got != postgres.DefaultVersion() {
		t.Fatalf("Postgres binding = %q, want %q", got, postgres.DefaultVersion())
	}
}

func TestApplyPvYmlServices_BindsMysqlDefaultVersion(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	stageMysqlBinary(t, mysql.DefaultVersion())

	reg := &registry.Registry{Projects: []registry.Project{{Name: "app", Path: t.TempDir()}}}
	ctx := &automation.Context{
		ProjectName: "app",
		Registry:    reg,
		ProjectConfig: &config.ProjectConfig{
			Mysql: &config.ServiceConfig{},
		},
	}

	_, err := (&ApplyPvYmlServicesStep{}).Run(ctx)
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if got := reg.Projects[0].Services.MySQL; got != mysql.DefaultVersion() {
		t.Fatalf("MySQL binding = %q, want %q", got, mysql.DefaultVersion())
	}
}
```

- [ ] **Step 2: Run pv.yml service tests and verify failure**

Run: `go test ./internal/automation/steps -run ApplyPvYmlServices`

Expected: FAIL because Postgres/MySQL currently require explicit versions and package default helpers do not exist.

- [ ] **Step 3: Add Postgres/MySQL default resolution helpers**

Add to `internal/postgres/version.go`:

```go
const defaultVersion = "18"

func DefaultVersion() string { return defaultVersion }

func ResolveVersion(version string) (string, error) {
	if version == "" {
		return DefaultVersion(), nil
	}
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return version, nil
}

func ValidateVersion(version string) error {
	switch version {
	case "17", "18":
		return nil
	default:
		return fmt.Errorf("unsupported postgres version %q (want one of 17, 18)", version)
	}
}
```

Add to `internal/mysql/version.go`:

```go
const defaultVersion = "8.4"

func DefaultVersion() string { return defaultVersion }

func ResolveVersion(version string) (string, error) {
	if version == "" {
		return DefaultVersion(), nil
	}
	if err := ValidateVersion(version); err != nil {
		return "", err
	}
	return version, nil
}

func ValidateVersion(version string) error {
	if !binaries.IsValidMysqlVersion(version) {
		return fmt.Errorf("unsupported mysql version %q (want one of 8.0, 8.4, 9.7)", version)
	}
	return nil
}
```

Import `github.com/prvious/pv/internal/binaries` in `internal/mysql/version.go`.

Update `internal/commands/postgres/install.go` so the command uses package resolution:

```go
RunE: func(cmd *cobra.Command, args []string) error {
	majorArg := ""
	if len(args) > 0 {
		majorArg = args[0]
	}
	major, err := pg.ResolveVersion(majorArg)
	if err != nil {
		return err
	}

	if pg.IsInstalled(major) {
		if err := pg.EnsureRuntime(major); err != nil {
			return err
		}
		if err := pg.SetWanted(major, pg.WantedRunning); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("PostgreSQL %s already installed — marked as wanted running.", major))
		return signalDaemon()
	}
	if err := downloadCmd.RunE(downloadCmd, []string{major}); err != nil {
		return err
	}
	ui.Success(fmt.Sprintf("PostgreSQL %s installed.", major))
	return signalDaemon()
},
```

Update `internal/commands/mysql/install.go` so the command uses package resolution:

```go
RunE: func(cmd *cobra.Command, args []string) error {
	versionArg := ""
	if len(args) > 0 {
		versionArg = args[0]
	}
	version, err := my.ResolveVersion(versionArg)
	if err != nil {
		return err
	}

	if my.IsInstalled(version) {
		if err := my.SetWanted(version, my.WantedRunning); err != nil {
			return err
		}
		ui.Success(fmt.Sprintf("MySQL %s already installed — marked as wanted running.", version))
		return signalDaemon()
	}
	if err := downloadCmd.RunE(downloadCmd, []string{version}); err != nil {
		return err
	}
	ui.Success(fmt.Sprintf("MySQL %s installed.", version))
	return signalDaemon()
},
```

- [ ] **Step 4: Resolve omitted versions in ApplyPvYmlServicesStep**

Change Postgres and MySQL blocks to:

```go
if cfg.Postgresql != nil {
	major, err := postgres.ResolveVersion(cfg.Postgresql.Version)
	if err != nil {
		return "", err
	}
	if !postgres.IsInstalled(major) {
		return "", fmt.Errorf("pv.yml postgresql %q is not installed — run `pv postgres:install %s`", major, major)
	}
	bindProjectPostgres(ctx.Registry, ctx.ProjectName, major)
	count++
}

if cfg.Mysql != nil {
	version, err := mysql.ResolveVersion(cfg.Mysql.Version)
	if err != nil {
		return "", err
	}
	if !mysql.IsInstalled(version) {
		return "", fmt.Errorf("pv.yml mysql %q is not installed — run `pv mysql:install %s`", version, version)
	}
	bindProjectMysql(ctx.Registry, ctx.ProjectName, version)
	count++
}
```

Update `internal/config/pvyml.go` `ServiceConfig` comment to state: every service block may omit `version`; omitted versions resolve to the current pv default for that service.

- [ ] **Step 5: Run pv.yml service tests and verify pass**

Run: `go test ./internal/automation/steps -run ApplyPvYmlServices`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add internal/postgres/version.go internal/mysql/version.go internal/commands/postgres/install.go internal/commands/mysql/install.go internal/config/pvyml.go internal/automation/steps/apply_pvyml_services.go internal/automation/steps/apply_pvyml_services_test.go
git commit -m "refactor(pvyml): default omitted service versions"
```

---

### Task 7: Remove Remaining `latest` Assumptions Across Orchestrators

**Files:**
- Modify: `cmd/update.go`
- Modify: `cmd/uninstall.go`
- Modify: `cmd/setup.go`
- Modify: `internal/commands/mailpit/start.go`
- Modify: `internal/commands/mailpit/stop.go`
- Modify: `internal/commands/mailpit/restart.go`
- Modify: `internal/commands/mailpit/install.go`
- Modify: `internal/commands/mailpit/update.go`
- Modify: `internal/commands/mailpit/uninstall.go`
- Modify: `internal/commands/rustfs/start.go`
- Modify: `internal/commands/rustfs/stop.go`
- Modify: `internal/commands/rustfs/restart.go`
- Modify: `internal/commands/rustfs/install.go`
- Modify: `internal/commands/rustfs/update.go`
- Modify: `internal/commands/rustfs/uninstall.go`
- Modify: `internal/caddy/caddy.go`
- Modify: `internal/server/manager.go`
- Modify tests under `cmd/`, `internal/server/`, and `internal/registry/` that assert `latest` for Mailpit/RustFS.

- [ ] **Step 1: Search for stale assumptions**

Run: `rg 'mailpit-latest|rustfs-latest|Mail: "latest"|S3: "latest"|ServiceDataDir\("mail"|ServiceDataDir\("s3"|InternalBinDir\(\).*mailpit|InternalBinDir\(\).*rustfs'`

Expected: matches appear in tests and production code before cleanup.

- [ ] **Step 2: Update expected process names and bindings**

Replace expected process names:

```text
mailpit-latest        -> mailpit-1
rustfs-latest         -> rustfs-1.0.0-beta
Mail: "latest"        -> Mail: mailpit.DefaultVersion()
S3: "latest"          -> S3: rustfs.DefaultVersion()
```

In tests that cannot import the package without an import cycle, use concrete strings `"1"` and `"1.0.0-beta"`.

- [ ] **Step 3: Update Caddy route enabling only if necessary**

`internal/caddy/caddy.go` already calls `DefaultVersion()`. Keep that pattern. Only update tests or comments that mention singleton/latest behavior.

- [ ] **Step 4: Update global update/uninstall behavior only if tests fail**

`cmd/update.go` already enumerates `mailpit.InstalledVersions()` and `rustfs.InstalledVersions()`. Keep that behavior.

`cmd/uninstall.go` checks `DefaultVersion()` for Mailpit/RustFS. That remains correct while each service supports one line.

- [ ] **Step 5: Normalize Mailpit/RustFS lifecycle command signaling**

Apply this policy to Mailpit and RustFS command packages:

```text
install   -> final signalDaemon after successful install/config generation
start     -> final signalDaemon after wanted-running is saved
stop      -> final signalDaemon after wanted-stopped is saved
restart   -> pre-stop server.SignalDaemon when daemon is running, final signalDaemon after wanted-running is saved
update    -> pre-stop server.SignalDaemon when daemon is running, final signalDaemon after successful update
uninstall -> pre-stop server.SignalDaemon when daemon is running, final signalDaemon after successful cleanup/unbind/config generation
```

Keep the pre-stop `server.IsRunning()` checks before waits. Those gates avoid waiting on a daemon that is not running. The final signal should always route through the package's `signalDaemon(pkg.DisplayName())` helper so no-daemon behavior stays consistent.

In `internal/commands/mailpit/update.go`, replace the final daemon signal block:

```go
ui.Success(fmt.Sprintf("%s %s updated.", pkg.DisplayName(), resolved))
if server.IsRunning() {
	return server.SignalDaemon()
}
return nil
```

with:

```go
ui.Success(fmt.Sprintf("%s %s updated.", pkg.DisplayName(), resolved))
return signalDaemon(pkg.DisplayName())
```

In `internal/commands/rustfs/update.go`, make the same replacement. Keep the earlier pre-update `server.IsRunning()` check before waiting for the process to stop; only the final post-update signal should always route through `signalDaemon`.

In `internal/commands/mailpit/stop.go` and `internal/commands/rustfs/stop.go`, replace final direct `server.IsRunning()` signal blocks with `return signalDaemon(pkg.DisplayName())`.

In `internal/commands/mailpit/restart.go` and `internal/commands/rustfs/restart.go`, keep the first stop signal gated by `server.IsRunning()` before `WaitStopped`, then use `return signalDaemon(pkg.DisplayName())` after wanted-running is saved.

In `internal/commands/mailpit/uninstall.go` and `internal/commands/rustfs/uninstall.go`, after successful `pkg.Uninstall`, registry fallback/unbind behavior, and `caddy.GenerateServiceSiteConfigs()`, finish with `return signalDaemon(pkg.DisplayName())` instead of returning nil. If the service was not installed and the command exits early without changing state, no signal is needed.

- [ ] **Step 6: Run targeted tests**

Run:

```bash
go test ./internal/commands/mailpit ./internal/commands/rustfs ./internal/server ./internal/caddy ./internal/registry ./cmd -run 'Mailpit|Rustfs|RustFS|Services|Update|Uninstall'
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add cmd internal/commands/mailpit internal/commands/rustfs internal/server internal/caddy internal/registry
git commit -m "refactor(services): remove latest service identity assumptions"
```

---

### Task 8: Final Verification And Documentation Sync

**Files:**
- Modify: `docs/superpowers/specs/2026-05-13-unified-managed-service-lifecycle-design.md` only if implementation reveals a mismatch.
- Modify: `README.md` only if it documents Mailpit/RustFS `latest` commands or paths.

- [ ] **Step 1: Run stale-string search**

Run: `rg 'mailpit-latest|rustfs-latest|"latest"' internal cmd .github/workflows docs README.md`

Expected: remaining `latest` matches are unrelated to Mailpit/RustFS service identity, such as self-update or upstream-release wording.

- [ ] **Step 2: Run formatting**

Run: `gofmt -w .`

Expected: command exits 0.

- [ ] **Step 3: Run vet**

Run: `go vet ./...`

Expected: PASS.

- [ ] **Step 4: Run build**

Run: `go build ./...`

Expected: PASS.

- [ ] **Step 5: Run tests**

Run: `go test ./...`

Expected: PASS.

- [ ] **Step 6: Commit final cleanup**

```bash
git add .
git commit -m "test: verify versioned service archive parity"
```

---

## Follow-Up Plan

After this plan lands, create a second implementation plan for shared helper extraction:

- `internal/servicestate`
- `internal/serviceartifact`
- `internal/servicewait`
- `internal/servicecmd`
- `internal/servicereconcile`

Do not extract those helpers during this plan unless a tiny local helper is needed to keep Mailpit/RustFS code readable. The goal here is working parity first.
