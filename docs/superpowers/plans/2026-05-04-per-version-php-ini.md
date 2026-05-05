# Per-version php.ini Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each installed PHP version gets a writable `~/.pv/php/<ver>/etc/php.ini` (upstream `php.ini-development` verbatim) and a `conf.d/` scan dir with a pv-managed `00-pv.ini`. Both the `php` CLI shim and the FrankenPHP launcher set `PHPRC` and `PHP_INI_SCAN_DIR` so the same per-version ini is loaded by both.

**Architecture:** Pure runtime wiring — no static-php-cli build flags. The two callers that exec PHP (the bash shim and `cmd.Env` in the FrankenPHP Go launcher) export `PHPRC=~/.pv/php/<ver>/etc` and `PHP_INI_SCAN_DIR=~/.pv/php/<ver>/conf.d`. Upstream `php.ini-development` is bundled into the existing `php-mac-*.tar.gz` artifact. A new `phpenv.EnsureIniLayout(version)` provisions the dirs, copies `php.ini` from the template if absent, and (re)writes `00-pv.ini` with pv's path defaults.

**Tech Stack:** Go (cobra CLI, standard library), bash (shim + e2e), GitHub Actions YAML, static-php-cli (build-time only — no flag changes).

**Spec:** `docs/superpowers/specs/2026-05-04-per-version-php-ini-design.md`

---

## File Structure

**New:**
- `internal/phpenv/inilayout.go` — `EnsureIniLayout` and `00-pv.ini` renderer.
- `internal/phpenv/inilayout_test.go` — unit tests.
- `internal/phpenv/testdata/php.ini-development` — small fixture.
- `internal/server/frankenphp_test.go` — env-wiring tests.
- `scripts/e2e/php-ini.sh` — e2e phase.

**Modified:**
- `internal/config/paths.go` (+ `paths_test.go`) — `PhpEtcDir`, `PhpConfDDir`, `PhpSessionDir`, `PhpTmpDir`, `PhpEnv`.
- `internal/binaries/download.go` — add `ErrEntryNotFound` sentinel.
- `internal/phpenv/install.go` — extract `php.ini-development`; call `EnsureIniLayout`.
- `internal/phpenv/phpenv.go` — `EnsureInstalled` calls `EnsureIniLayout` on the already-installed branch.
- `internal/tools/shims.go` (+ `tool_test.go`) — shim exports the two env vars.
- `internal/server/frankenphp.go` — pass `config.PhpEnv(version)` into `cmd.Env`; resolve global version in `StartFrankenPHP`.
- `internal/server/process.go` — backfill: walk `InstalledVersions()` and call `EnsureIniLayout` at daemon start.
- `.github/workflows/build-artifacts.yml` — bundle `php.ini-development` in the PHP CLI tarball.
- `.github/workflows/e2e.yml` — wire the new php-ini phase.

---

## Task 1: Add config helpers for per-version paths and env

**Files:**
- Modify: `internal/config/paths.go`
- Modify: `internal/config/paths_test.go`

- [ ] **Step 1: Add the failing tests**

Append to `internal/config/paths_test.go`:

```go
func TestPhpEtcDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpEtcDir("8.4")
	want := filepath.Join(home, ".pv", "php", "8.4", "etc")
	if got != want {
		t.Errorf("PhpEtcDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpConfDDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpConfDDir("8.4")
	want := filepath.Join(home, ".pv", "php", "8.4", "conf.d")
	if got != want {
		t.Errorf("PhpConfDDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpSessionDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpSessionDir("8.4")
	want := filepath.Join(home, ".pv", "data", "sessions", "8.4")
	if got != want {
		t.Errorf("PhpSessionDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpTmpDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpTmpDir("8.4")
	want := filepath.Join(home, ".pv", "data", "tmp", "8.4")
	if got != want {
		t.Errorf("PhpTmpDir(\"8.4\") = %q, want %q", got, want)
	}
}

func TestPhpEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := PhpEnv("8.4")
	wantPHPRC := "PHPRC=" + filepath.Join(home, ".pv", "php", "8.4", "etc")
	wantScan := "PHP_INI_SCAN_DIR=" + filepath.Join(home, ".pv", "php", "8.4", "conf.d")

	if len(got) != 2 {
		t.Fatalf("PhpEnv() returned %d entries, want 2", len(got))
	}
	if got[0] != wantPHPRC {
		t.Errorf("PhpEnv()[0] = %q, want %q", got[0], wantPHPRC)
	}
	if got[1] != wantScan {
		t.Errorf("PhpEnv()[1] = %q, want %q", got[1], wantScan)
	}
}
```

- [ ] **Step 2: Run tests, verify they fail**

```bash
go test ./internal/config/ -run 'TestPhpEtcDir|TestPhpConfDDir|TestPhpSessionDir|TestPhpTmpDir|TestPhpEnv' -v
```

Expected: FAIL — undefined: `PhpEtcDir`, `PhpConfDDir`, `PhpSessionDir`, `PhpTmpDir`, `PhpEnv`.

- [ ] **Step 3: Add the helpers**

In `internal/config/paths.go`, add immediately after the existing `PhpVersionDir`:

```go
func PhpEtcDir(version string) string {
	return filepath.Join(PhpVersionDir(version), "etc")
}

func PhpConfDDir(version string) string {
	return filepath.Join(PhpVersionDir(version), "conf.d")
}

func PhpSessionDir(version string) string {
	return filepath.Join(DataDir(), "sessions", version)
}

func PhpTmpDir(version string) string {
	return filepath.Join(DataDir(), "tmp", version)
}

// PhpEnv returns env vars that point a PHP/FrankenPHP process at the
// per-version php.ini and conf.d. Caller must pass a non-empty version.
func PhpEnv(version string) []string {
	return []string{
		"PHPRC=" + PhpEtcDir(version),
		"PHP_INI_SCAN_DIR=" + PhpConfDDir(version),
	}
}
```

- [ ] **Step 4: Run tests, verify they pass**

```bash
go test ./internal/config/ -v
```

Expected: PASS.

- [ ] **Step 5: gofmt, vet, build**

```bash
gofmt -w internal/config/
go vet ./internal/config/
go build ./...
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add internal/config/paths.go internal/config/paths_test.go
git commit -m "config: add per-version php paths and PHPRC env helpers"
```

---

## Task 2: Add ErrEntryNotFound sentinel to binaries

**Files:**
- Modify: `internal/binaries/download.go`
- Modify: `internal/binaries/download_test.go` (or create if missing)

Allows callers to distinguish "tar entry missing" from "I/O error" — needed by the install path so old artifacts (built before this feature) don't break extraction.

- [ ] **Step 1: Check whether the test file exists**

```bash
ls internal/binaries/
```

If `download_test.go` doesn't exist, create it. If it does, append.

- [ ] **Step 2: Add the failing test**

Append to (or create) `internal/binaries/download_test.go`:

```go
package binaries

import (
	"archive/tar"
	"compress/gzip"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

// makeTarGz writes a single-file tarball at archivePath containing entry
// `entryName` with the given content. Used to keep tests hermetic.
func makeTarGz(t *testing.T, archivePath, entryName, content string) {
	t.Helper()
	f, err := os.Create(archivePath)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	gz := gzip.NewWriter(f)
	tw := tar.NewWriter(gz)
	hdr := &tar.Header{Name: entryName, Mode: 0644, Size: int64(len(content)), Typeflag: tar.TypeReg}
	if err := tw.WriteHeader(hdr); err != nil {
		t.Fatal(err)
	}
	if _, err := tw.Write([]byte(content)); err != nil {
		t.Fatal(err)
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if err := gz.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestExtractTarGz_EntryNotFound(t *testing.T) {
	dir := t.TempDir()
	archive := filepath.Join(dir, "a.tar.gz")
	makeTarGz(t, archive, "php", "binary content")

	err := ExtractTarGz(archive, filepath.Join(dir, "out"), "php.ini-development")
	if err == nil {
		t.Fatal("ExtractTarGz returned nil for missing entry, want error")
	}
	if !errors.Is(err, ErrEntryNotFound) {
		t.Errorf("ExtractTarGz error = %v, want errors.Is(err, ErrEntryNotFound)", err)
	}
}

func TestExtractTarGz_EntryFound(t *testing.T) {
	dir := t.TempDir()
	archive := filepath.Join(dir, "a.tar.gz")
	makeTarGz(t, archive, "php", "hello")

	dest := filepath.Join(dir, "out")
	if err := ExtractTarGz(archive, dest, "php"); err != nil {
		t.Fatalf("ExtractTarGz error = %v", err)
	}

	got, err := os.ReadFile(dest)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "hello" {
		t.Errorf("extracted content = %q, want %q", string(got), "hello")
	}
}
```

- [ ] **Step 3: Run tests, verify EntryNotFound test fails**

```bash
go test ./internal/binaries/ -run TestExtractTarGz_EntryNotFound -v
```

Expected: FAIL — `undefined: ErrEntryNotFound` (compile error).

- [ ] **Step 4: Add the sentinel and wire it into ExtractTarGz**

In `internal/binaries/download.go`, ensure `errors` is imported. Add near the top of the file (after imports, before the first function):

```go
// ErrEntryNotFound is returned by ExtractTarGz when the requested entry
// is not present in the archive. Callers can use errors.Is to tolerate
// optional entries (e.g. files added in newer artifact builds).
var ErrEntryNotFound = errors.New("entry not found in archive")
```

Then change the final `return` of `ExtractTarGz` from:

```go
return fmt.Errorf("binary %q not found in archive", binaryName)
```

to:

```go
return fmt.Errorf("%q: %w", binaryName, ErrEntryNotFound)
```

- [ ] **Step 5: Run tests, verify both pass**

```bash
go test ./internal/binaries/ -v
```

Expected: PASS.

- [ ] **Step 6: gofmt, vet, build**

```bash
gofmt -w internal/binaries/
go vet ./internal/binaries/
go build ./...
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add internal/binaries/download.go internal/binaries/download_test.go
git commit -m "binaries: add ErrEntryNotFound sentinel for ExtractTarGz"
```

---

## Task 3: phpenv.EnsureIniLayout — provision dirs + 00-pv.ini

**Files:**
- Create: `internal/phpenv/inilayout.go`
- Create: `internal/phpenv/inilayout_test.go`
- Create: `internal/phpenv/testdata/php.ini-development`

The one piece of new behaviour. Must be idempotent, must not clobber `etc/php.ini` if it exists, must always (re)write `conf.d/00-pv.ini`.

- [ ] **Step 1: Create the testdata fixture**

`internal/phpenv/testdata/php.ini-development` (small, opaque to logic):

```ini
; pv test fixture — stand-in for upstream php.ini-development
memory_limit = 128M
display_errors = On
```

- [ ] **Step 2: Write the failing tests**

Create `internal/phpenv/inilayout_test.go`:

```go
package phpenv

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// seedIniDevelopment drops the testdata fixture into etc/php.ini-development
// for the given version, mirroring what the install code does.
func seedIniDevelopment(t *testing.T, version string) {
	t.Helper()
	src, err := os.ReadFile("testdata/php.ini-development")
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	dst := filepath.Join(config.PhpEtcDir(version), "php.ini-development")
	if err := os.MkdirAll(filepath.Dir(dst), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(dst, src, 0644); err != nil {
		t.Fatal(err)
	}
}

func TestEnsureIniLayout_CreatesAllDirsAndFiles(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	// Dirs.
	for _, dir := range []string{
		config.PhpEtcDir("8.4"),
		config.PhpConfDDir("8.4"),
		config.PhpSessionDir("8.4"),
		config.PhpTmpDir("8.4"),
	} {
		info, err := os.Stat(dir)
		if err != nil {
			t.Errorf("dir %s missing: %v", dir, err)
			continue
		}
		if !info.IsDir() {
			t.Errorf("%s exists but is not a dir", dir)
		}
	}

	// php.ini was copied from php.ini-development.
	iniPath := filepath.Join(config.PhpEtcDir("8.4"), "php.ini")
	got, err := os.ReadFile(iniPath)
	if err != nil {
		t.Fatalf("read php.ini: %v", err)
	}
	if !strings.Contains(string(got), "memory_limit = 128M") {
		t.Errorf("php.ini does not contain fixture content; got: %q", string(got))
	}

	// 00-pv.ini was written and contains the expected directives.
	pvIniPath := filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini")
	pvIni, err := os.ReadFile(pvIniPath)
	if err != nil {
		t.Fatalf("read 00-pv.ini: %v", err)
	}
	wantSession := "session.save_path = \"" + config.PhpSessionDir("8.4") + "\""
	if !strings.Contains(string(pvIni), wantSession) {
		t.Errorf("00-pv.ini missing %q; got:\n%s", wantSession, string(pvIni))
	}
	if !strings.Contains(string(pvIni), "date.timezone = UTC") {
		t.Error("00-pv.ini missing date.timezone")
	}
}

func TestEnsureIniLayout_PreservesExistingPhpIni(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	// Pre-create php.ini with user content; EnsureIniLayout must not overwrite.
	iniPath := filepath.Join(config.PhpEtcDir("8.4"), "php.ini")
	if err := os.MkdirAll(filepath.Dir(iniPath), 0755); err != nil {
		t.Fatal(err)
	}
	userContent := "; user-edited php.ini\nmemory_limit = 1G\n"
	if err := os.WriteFile(iniPath, []byte(userContent), 0644); err != nil {
		t.Fatal(err)
	}

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	got, err := os.ReadFile(iniPath)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != userContent {
		t.Errorf("php.ini was clobbered; got:\n%s\nwant:\n%s", string(got), userContent)
	}
}

func TestEnsureIniLayout_RegeneratesPvIni(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	// Pre-create 00-pv.ini with stale content; EnsureIniLayout must overwrite.
	pvIniPath := filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini")
	if err := os.MkdirAll(filepath.Dir(pvIniPath), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(pvIniPath, []byte("; stale junk\n"), 0644); err != nil {
		t.Fatal(err)
	}

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	got, err := os.ReadFile(pvIniPath)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(got), "stale junk") {
		t.Errorf("00-pv.ini was not regenerated; got:\n%s", string(got))
	}
	if !strings.Contains(string(got), "date.timezone = UTC") {
		t.Errorf("regenerated 00-pv.ini missing canonical content; got:\n%s", string(got))
	}
}

func TestEnsureIniLayout_Idempotent(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	seedIniDevelopment(t, "8.4")

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("first EnsureIniLayout error = %v", err)
	}
	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("second EnsureIniLayout error = %v", err)
	}
}

func TestEnsureIniLayout_NoIniDevelopmentSource(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}
	// Deliberately do NOT seed php.ini-development — simulating an old install.

	if err := EnsureIniLayout("8.4"); err != nil {
		t.Fatalf("EnsureIniLayout error = %v", err)
	}

	// Dirs and 00-pv.ini still created.
	if _, err := os.Stat(config.PhpConfDDir("8.4")); err != nil {
		t.Errorf("conf.d not created: %v", err)
	}
	if _, err := os.Stat(filepath.Join(config.PhpConfDDir("8.4"), "00-pv.ini")); err != nil {
		t.Errorf("00-pv.ini not written: %v", err)
	}
	// But php.ini is NOT created (no source to copy from).
	iniPath := filepath.Join(config.PhpEtcDir("8.4"), "php.ini")
	if _, err := os.Stat(iniPath); !os.IsNotExist(err) {
		t.Errorf("php.ini should not exist when source is missing; got err=%v", err)
	}
}
```

- [ ] **Step 3: Run tests, verify they fail**

```bash
go test ./internal/phpenv/ -run TestEnsureIniLayout -v
```

Expected: FAIL — `undefined: EnsureIniLayout`.

- [ ] **Step 4: Implement EnsureIniLayout**

Create `internal/phpenv/inilayout.go`:

```go
package phpenv

import (
	"fmt"
	"io"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
)

// pvIniHeader is the header written into every conf.d/00-pv.ini.
const pvIniHeader = `; Managed by pv — regenerated on every ` + "`pv php:install`" + ` / ` + "`pv php:update`" + `.
; For your own overrides, create a sibling file like 99-local.ini —
; conf.d files load alphabetically and later files win.
`

// EnsureIniLayout provisions the per-version ini directory layout under
// ~/.pv/php/<version>/. It is idempotent and safe to call repeatedly:
//
//   - Creates etc/, conf.d/, ~/.pv/data/sessions/<version>/, and
//     ~/.pv/data/tmp/<version>/.
//   - If etc/php.ini does not exist AND etc/php.ini-development exists,
//     copies the latter to the former. Existing etc/php.ini is preserved.
//   - Always (re)writes conf.d/00-pv.ini with pv's path defaults for the
//     given version. This file is pv-managed.
func EnsureIniLayout(version string) error {
	dirs := []string{
		config.PhpEtcDir(version),
		config.PhpConfDDir(version),
		config.PhpSessionDir(version),
		config.PhpTmpDir(version),
	}
	for _, d := range dirs {
		if err := os.MkdirAll(d, 0755); err != nil {
			return fmt.Errorf("create %s: %w", d, err)
		}
	}

	if err := seedPhpIniIfMissing(version); err != nil {
		return err
	}

	return writePvIni(version)
}

// seedPhpIniIfMissing copies etc/php.ini-development to etc/php.ini if and
// only if php.ini does not yet exist. Missing source is a no-op (older
// artifacts didn't bundle the template).
func seedPhpIniIfMissing(version string) error {
	target := filepath.Join(config.PhpEtcDir(version), "php.ini")
	if _, err := os.Stat(target); err == nil {
		return nil // user file present; never touch.
	}

	source := filepath.Join(config.PhpEtcDir(version), "php.ini-development")
	if _, err := os.Stat(source); os.IsNotExist(err) {
		return nil // older artifact, nothing to copy.
	} else if err != nil {
		return fmt.Errorf("stat %s: %w", source, err)
	}

	in, err := os.Open(source)
	if err != nil {
		return fmt.Errorf("open %s: %w", source, err)
	}
	defer in.Close()

	out, err := os.OpenFile(target, os.O_CREATE|os.O_WRONLY|os.O_EXCL, 0644)
	if err != nil {
		return fmt.Errorf("create %s: %w", target, err)
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return fmt.Errorf("copy %s -> %s: %w", source, target, err)
	}
	return out.Close()
}

// writePvIni renders and writes conf.d/00-pv.ini. Always overwrites.
func writePvIni(version string) error {
	body := pvIniHeader + fmt.Sprintf(`
date.timezone = UTC

session.save_path = %q
sys_temp_dir     = %q
upload_tmp_dir   = %q
`,
		config.PhpSessionDir(version),
		config.PhpTmpDir(version),
		config.PhpTmpDir(version),
	)

	path := filepath.Join(config.PhpConfDDir(version), "00-pv.ini")
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}
```

- [ ] **Step 5: Run tests, verify they pass**

```bash
go test ./internal/phpenv/ -run TestEnsureIniLayout -v
```

Expected: PASS for all five subtests.

- [ ] **Step 6: gofmt, vet, build, run full phpenv tests**

```bash
gofmt -w internal/phpenv/
go vet ./internal/phpenv/
go build ./...
go test ./internal/phpenv/
```

Expected: all clean and PASS.

- [ ] **Step 7: Commit**

```bash
git add internal/phpenv/inilayout.go internal/phpenv/inilayout_test.go internal/phpenv/testdata/php.ini-development
git commit -m "phpenv: add EnsureIniLayout for per-version php.ini and conf.d"
```

---

## Task 4: Wire EnsureIniLayout into install + extract php.ini-development

**Files:**
- Modify: `internal/phpenv/install.go`
- Modify: `internal/phpenv/phpenv.go`

`InstallProgress` extracts `php.ini-development` from the tarball (best-effort; tolerates absence for older artifacts) and calls `EnsureIniLayout`. `EnsureInstalled` covers the already-installed branch so backfill works without re-downloading.

- [ ] **Step 1: Modify InstallProgress to extract ini-development and call EnsureIniLayout**

In `internal/phpenv/install.go`, replace the existing PHP CLI extraction block. The current code is:

```go
if err := binaries.ExtractTarGz(phpArchive, phpDest, "php"); err != nil {
    return fmt.Errorf("extract PHP CLI: %w", err)
}
os.Remove(phpArchive)

if err := binaries.MakeExecutable(phpDest); err != nil {
    return err
}

return nil
```

Replace with:

```go
if err := binaries.ExtractTarGz(phpArchive, phpDest, "php"); err != nil {
    return fmt.Errorf("extract PHP CLI: %w", err)
}

// Extract the upstream php.ini-development template if present.
// Older artifacts (built before the per-version ini work) don't bundle
// it; tolerate that — EnsureIniLayout handles a missing source gracefully.
iniDevDest := filepath.Join(config.PhpEtcDir(phpVersion), "php.ini-development")
if err := os.MkdirAll(filepath.Dir(iniDevDest), 0755); err != nil {
    return fmt.Errorf("create etc dir: %w", err)
}
if err := binaries.ExtractTarGz(phpArchive, iniDevDest, "php.ini-development"); err != nil {
    if !errors.Is(err, binaries.ErrEntryNotFound) {
        return fmt.Errorf("extract php.ini-development: %w", err)
    }
    // Older artifact — silently continue; EnsureIniLayout will skip the copy.
}

os.Remove(phpArchive)

if err := binaries.MakeExecutable(phpDest); err != nil {
    return err
}

return EnsureIniLayout(phpVersion)
```

Add `"errors"` to the imports if not already present.

- [ ] **Step 2: Modify EnsureInstalled to backfill on the already-installed branch**

In `internal/phpenv/phpenv.go`, the current `EnsureInstalled` is:

```go
func EnsureInstalled(version string) error {
	if IsInstalled(version) {
		return nil
	}
	client := &http.Client{Timeout: 5 * time.Minute}
	if err := InstallProgress(client, version, nil); err != nil {
		return fmt.Errorf("install PHP %s: %w", version, err)
	}
	return nil
}
```

Replace with:

```go
func EnsureInstalled(version string) error {
	if IsInstalled(version) {
		// Backfill the ini layout for installs that predate this feature.
		// EnsureIniLayout is idempotent — cheap to call on every check.
		return EnsureIniLayout(version)
	}
	client := &http.Client{Timeout: 5 * time.Minute}
	if err := InstallProgress(client, version, nil); err != nil {
		return fmt.Errorf("install PHP %s: %w", version, err)
	}
	return nil
}
```

- [ ] **Step 3: gofmt, vet, build**

```bash
gofmt -w internal/phpenv/
go vet ./internal/phpenv/
go build ./...
```

Expected: clean.

- [ ] **Step 4: Run phpenv tests**

```bash
go test ./internal/phpenv/
```

Expected: PASS — existing tests untouched, EnsureIniLayout tests still pass. (No new test for the install change here — extraction is exercised end-to-end in the e2e phase from Task 9, since unit-testing it requires mocking HTTP downloads.)

- [ ] **Step 5: Commit**

```bash
git add internal/phpenv/install.go internal/phpenv/phpenv.go
git commit -m "phpenv: extract php.ini-development on install and backfill on EnsureInstalled"
```

---

## Task 5: Update php shim to export PHPRC and PHP_INI_SCAN_DIR

**Files:**
- Modify: `internal/tools/shims.go`
- Modify: `internal/tools/tool_test.go`

- [ ] **Step 1: Add the failing test**

Append to `internal/tools/tool_test.go`:

```go
func TestPhpShim_ExportsPhpEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	if err := config.EnsureDirs(); err != nil {
		t.Fatal(err)
	}

	if err := Expose(Get("php")); err != nil {
		t.Fatalf("Expose(php) error = %v", err)
	}

	content, err := os.ReadFile(filepath.Join(config.BinDir(), "php"))
	if err != nil {
		t.Fatalf("read shim: %v", err)
	}
	got := string(content)

	if !strings.Contains(got, `export PHPRC="$PV_PHP_DIR/$VERSION/etc"`) {
		t.Errorf("php shim missing PHPRC export; content:\n%s", got)
	}
	if !strings.Contains(got, `export PHP_INI_SCAN_DIR="$PV_PHP_DIR/$VERSION/conf.d"`) {
		t.Errorf("php shim missing PHP_INI_SCAN_DIR export; content:\n%s", got)
	}

	// The exec line must remain the last meaningful instruction so env
	// vars are inherited by the real binary.
	execIdx := strings.Index(got, `exec "$BINARY" "$@"`)
	if execIdx == -1 {
		t.Fatal("php shim missing exec line")
	}
	phprcIdx := strings.Index(got, "export PHPRC=")
	scanIdx := strings.Index(got, "export PHP_INI_SCAN_DIR=")
	if phprcIdx > execIdx || scanIdx > execIdx {
		t.Error("php shim exports must precede exec")
	}
}
```

- [ ] **Step 2: Run, verify it fails**

```bash
go test ./internal/tools/ -run TestPhpShim_ExportsPhpEnv -v
```

Expected: FAIL — both `Contains` assertions fail.

- [ ] **Step 3: Update the shim template**

In `internal/tools/shims.go`, replace `phpShimScript` with:

```go
const phpShimScript = `#!/bin/bash
# pv PHP version shim — delegates version resolution to pv binary.
set -euo pipefail

PV_PHP_DIR="%s"
PV_BIN="%s"

VERSION=$("$PV_BIN" php:current)
if [ -z "$VERSION" ]; then
    echo "pv: no PHP version configured. Run: pv php:install [version]" >&2
    exit 1
fi

BINARY="$PV_PHP_DIR/$VERSION/php"
if [ ! -x "$BINARY" ]; then
    echo "pv: PHP $VERSION is not installed. Run: pv php:install $VERSION" >&2
    exit 1
fi

export PHPRC="$PV_PHP_DIR/$VERSION/etc"
export PHP_INI_SCAN_DIR="$PV_PHP_DIR/$VERSION/conf.d"
exec "$BINARY" "$@"
`
```

(Only two `export` lines added before `exec`. `%s`-formatting and the rest are unchanged.)

- [ ] **Step 4: Run tests, verify pass**

```bash
go test ./internal/tools/ -v
```

Expected: PASS for all tests in the package.

- [ ] **Step 5: gofmt, vet, build**

```bash
gofmt -w internal/tools/
go vet ./internal/tools/
go build ./...
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add internal/tools/shims.go internal/tools/tool_test.go
git commit -m "tools: php shim exports PHPRC and PHP_INI_SCAN_DIR"
```

---

## Task 6: Wire env into FrankenPHP launcher

**Files:**
- Modify: `internal/server/frankenphp.go`
- Create: `internal/server/frankenphp_test.go`

`startFrankenPHPInstance` already has the `version` parameter. It also already builds `cmd.Env` from `config.CaddyEnv()`. We append `config.PhpEnv(version)` when `version != ""`. `StartFrankenPHP` currently passes `version=""` for the global instance — we resolve the global version from settings and pass it.

- [ ] **Step 1: Write failing tests for env construction**

Create `internal/server/frankenphp_test.go`:

```go
package server

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
)

// buildVersionCmd builds (without spawning) the *exec.Cmd that
// startFrankenPHPInstance would create, so we can inspect Env.
//
// We can't call startFrankenPHPInstance directly without spawning, so we
// extract the env-construction into a helper (frankenphpEnv) that returns
// the env slice given a version. The helper is what the production code
// uses internally.

func TestFrankenphpEnv_VersionedSetsPhpEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := frankenphpEnv("8.4")
	wantPHPRC := "PHPRC=" + filepath.Join(home, ".pv", "php", "8.4", "etc")
	wantScan := "PHP_INI_SCAN_DIR=" + filepath.Join(home, ".pv", "php", "8.4", "conf.d")

	if !contains(got, wantPHPRC) {
		t.Errorf("frankenphpEnv(\"8.4\") missing %q; got: %v", wantPHPRC, got)
	}
	if !contains(got, wantScan) {
		t.Errorf("frankenphpEnv(\"8.4\") missing %q; got: %v", wantScan, got)
	}

	// Should also still include CaddyEnv entries (XDG_DATA_HOME etc.).
	for _, want := range config.CaddyEnv() {
		if !contains(got, want) {
			t.Errorf("frankenphpEnv missing CaddyEnv entry %q", want)
		}
	}
}

func TestFrankenphpEnv_EmptyVersionOmitsPhpEnv(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	got := frankenphpEnv("")

	for _, e := range got {
		if strings.HasPrefix(e, "PHPRC=") || strings.HasPrefix(e, "PHP_INI_SCAN_DIR=") {
			t.Errorf("frankenphpEnv(\"\") leaked PHP env var: %q", e)
		}
	}
}

func contains(haystack []string, needle string) bool {
	for _, s := range haystack {
		if s == needle {
			return true
		}
	}
	return false
}
```

- [ ] **Step 2: Run, verify they fail**

```bash
go test ./internal/server/ -run TestFrankenphpEnv -v
```

Expected: FAIL — `undefined: frankenphpEnv`.

- [ ] **Step 3: Extract `frankenphpEnv` and use it in `startFrankenPHPInstance`**

In `internal/server/frankenphp.go`:

Add a new helper near the top of the file (after imports):

```go
// frankenphpEnv builds the env slice for a FrankenPHP child process.
// It always includes os.Environ + config.CaddyEnv. When version is
// non-empty, it also appends config.PhpEnv(version) so PHP loads the
// per-version php.ini and conf.d.
func frankenphpEnv(version string) []string {
	env := append(os.Environ(), config.CaddyEnv()...)
	if version != "" {
		env = append(env, config.PhpEnv(version)...)
	}
	return env
}
```

Then in `startFrankenPHPInstance`, replace:

```go
cmd.Env = append(os.Environ(), config.CaddyEnv()...)
```

with:

```go
cmd.Env = frankenphpEnv(version)
```

`Reload()` is intentionally left untouched — it spawns a short-lived `frankenphp reload` process that talks to the running daemon's admin API. It does not load PHP itself, so PHPRC/PHP_INI_SCAN_DIR on it would be inert.

Add `"github.com/prvious/pv/internal/phpenv"` to the imports (used in Step 4 below).

- [ ] **Step 4: Resolve global version in `StartFrankenPHP`**

Still in `internal/server/frankenphp.go`, replace `StartFrankenPHP`:

```go
func StartFrankenPHP() (*FrankenPHP, error) {
	return startFrankenPHPInstance(
		filepath.Join(config.BinDir(), "frankenphp"),
		config.CaddyfilePath(),
		config.CaddyStderrPath(),
		"http://localhost:2019/config/",
		"",
	)
}
```

with:

```go
func StartFrankenPHP() (*FrankenPHP, error) {
	// Resolve the global PHP version so the main FrankenPHP loads the
	// matching per-version php.ini. If unset (fresh install), pass empty
	// and let the binary fall back to its built-in defaults — other
	// startup paths already error on missing global PHP.
	globalVer, _ := phpenv.GlobalVersion()

	return startFrankenPHPInstance(
		filepath.Join(config.BinDir(), "frankenphp"),
		config.CaddyfilePath(),
		config.CaddyStderrPath(),
		"http://localhost:2019/config/",
		globalVer,
	)
}
```

- [ ] **Step 5: Run tests**

```bash
go test ./internal/server/ -v
```

Expected: PASS — `TestFrankenphpEnv_VersionedSetsPhpEnv` and `TestFrankenphpEnv_EmptyVersionOmitsPhpEnv` pass; existing server tests unaffected.

- [ ] **Step 6: gofmt, vet, build**

```bash
gofmt -w internal/server/
go vet ./internal/server/
go build ./...
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add internal/server/frankenphp.go internal/server/frankenphp_test.go
git commit -m "server: pass per-version PHPRC/PHP_INI_SCAN_DIR to FrankenPHP child"
```

---

## Task 7: Backfill EnsureIniLayout at daemon start

**Files:**
- Modify: `internal/server/process.go`

For users upgrading pv with PHP versions already installed, the daemon backfills `etc/`, `conf.d/`, and `00-pv.ini` so the env vars set in Task 6 point at populated dirs on the very first boot.

- [ ] **Step 1: Add the backfill call**

In `internal/server/process.go`, in `Start()`, after `settings, err := config.LoadSettings()` (around line 46) and before the registry load, add:

```go
// Backfill per-version ini layout for any installed PHP versions that
// predate this feature. EnsureIniLayout is idempotent and cheap.
// Failure is logged but non-fatal — a broken ini layout shouldn't block
// the daemon from starting; the affected version just won't load its ini.
if installed, err := phpenv.InstalledVersions(); err == nil {
    for _, v := range installed {
        if err := phpenv.EnsureIniLayout(v); err != nil {
            fmt.Fprintf(os.Stderr, "Warning: ini layout backfill for PHP %s failed: %v\n", v, err)
        }
    }
}
```

`phpenv` and `os`/`fmt` are already imported in process.go.

- [ ] **Step 2: gofmt, vet, build, run tests**

```bash
gofmt -w internal/server/
go vet ./internal/server/
go build ./...
go test ./...
```

Expected: clean across the board. Process-level tests don't exercise `Start()` directly (it spawns FrankenPHP), so this is covered by e2e in Task 9.

- [ ] **Step 3: Commit**

```bash
git add internal/server/process.go
git commit -m "server: backfill per-version ini layout at daemon start"
```

---

## Task 8: Bundle php.ini-development in the build-artifacts pipeline

**Files:**
- Modify: `.github/workflows/build-artifacts.yml`

The PHP CLI tarball must contain `php.ini-development` alongside the `php` binary so install can extract it. We assert presence in the source tree to fail loudly on a missing file.

- [ ] **Step 1: Update the "Package PHP CLI" step**

In `.github/workflows/build-artifacts.yml`, locate the existing step:

```yaml
      - name: Package PHP CLI
        run: |
          set -euo pipefail
          ARCH="${{ steps.arch.outputs.arch }}"
          PHP_BIN="dist/static-php-cli/buildroot/bin/php"
          chmod +x "$PHP_BIN"
          "$PHP_BIN" -v
          "$PHP_BIN" -m
          mkdir -p dist/cli-staging
          cp "$PHP_BIN" dist/cli-staging/php
          tar -C dist/cli-staging -czf "dist/php-mac-${ARCH}-php${{ matrix.php }}.tar.gz" php
          rm -rf dist/cli-staging
```

Replace with:

```yaml
      - name: Package PHP CLI
        run: |
          set -euo pipefail
          ARCH="${{ steps.arch.outputs.arch }}"
          PHP_BIN="dist/static-php-cli/buildroot/bin/php"
          INI_DEV="dist/static-php-cli/source/php-src/php.ini-development"
          chmod +x "$PHP_BIN"
          "$PHP_BIN" -v
          "$PHP_BIN" -m
          test -f "$INI_DEV" || { echo "::error::missing $INI_DEV — php-src layout may have changed"; exit 1; }
          mkdir -p dist/cli-staging
          cp "$PHP_BIN" dist/cli-staging/php
          cp "$INI_DEV" dist/cli-staging/php.ini-development
          tar -C dist/cli-staging -czf "dist/php-mac-${ARCH}-php${{ matrix.php }}.tar.gz" php php.ini-development

          # Sanity: verify both files extract back out cleanly.
          mkdir -p dist/cli-verify
          tar -C dist/cli-verify -xzf "dist/php-mac-${ARCH}-php${{ matrix.php }}.tar.gz"
          test -x dist/cli-verify/php
          test -s dist/cli-verify/php.ini-development
          rm -rf dist/cli-staging dist/cli-verify
```

- [ ] **Step 2: Verify the YAML is well-formed**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/build-artifacts.yml'))" 2>&1 || \
  ruby -ryaml -e "YAML.load_file('.github/workflows/build-artifacts.yml')"
```

If neither python3 nor ruby is available, skip. (Not required — GitHub will reject malformed YAML on push.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/build-artifacts.yml
git commit -m "ci: bundle php.ini-development in static PHP CLI tarball"
```

- [ ] **Step 4: Push the branch and dispatch the FrankenPHP-only build**

(Run only after the rest of the plan is committed and the branch is pushed.)

```bash
git push -u origin <branch>
gh workflow run build-artifacts.yml --ref <branch> \
  -f skip_postgres=true -f skip_mysql=true
```

Per CLAUDE.md: skip postgres/mysql since this change only affects the FrankenPHP family.

Wait for the run to complete and confirm:
- The `frankenphp` matrix passes for all 3 PHP versions.
- The "Package PHP CLI" sanity check (extract + presence test) passes.

```bash
gh run list --workflow build-artifacts.yml --branch <branch> --limit 5
gh run watch <run-id>
```

---

## Task 9: E2E test phase

**Files:**
- Create: `scripts/e2e/php-ini.sh`
- Modify: `.github/workflows/e2e.yml`

End-to-end verification that the CLI shim and FrankenPHP both load the per-version ini, and that a `99-local.ini` drop-in overrides defaults as expected.

- [ ] **Step 1: Create the e2e script**

Create `scripts/e2e/php-ini.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &> /dev/null && pwd)
# shellcheck source=helpers.sh
source "$SCRIPT_DIR/helpers.sh"

PHP_VERSION="${PHP_VERSION:-8.4}"
ETC_DIR="$HOME/.pv/php/$PHP_VERSION/etc"
CONFD_DIR="$HOME/.pv/php/$PHP_VERSION/conf.d"

echo "==> Verify per-version ini layout for PHP $PHP_VERSION"
test -d "$ETC_DIR"   || { echo "FAIL: $ETC_DIR missing"; exit 1; }
test -d "$CONFD_DIR" || { echo "FAIL: $CONFD_DIR missing"; exit 1; }
test -s "$ETC_DIR/php.ini" || { echo "FAIL: $ETC_DIR/php.ini missing or empty"; exit 1; }
test -s "$ETC_DIR/php.ini-development" || { echo "FAIL: $ETC_DIR/php.ini-development missing"; exit 1; }
test -s "$CONFD_DIR/00-pv.ini" || { echo "FAIL: $CONFD_DIR/00-pv.ini missing"; exit 1; }
echo "  OK: layout present"

echo "==> Verify CLI loads the per-version ini"
LOADED=$(php -r 'echo php_ini_loaded_file();')
echo "  Loaded ini: $LOADED"
if [ "$LOADED" != "$ETC_DIR/php.ini" ]; then
    echo "FAIL: php loaded $LOADED, expected $ETC_DIR/php.ini"
    exit 1
fi

echo "==> Verify CLI scans the per-version conf.d"
SCANNED=$(php -r 'echo php_ini_scanned_files();')
echo "  Scanned: $SCANNED"
echo "$SCANNED" | grep -q "00-pv.ini" || { echo "FAIL: 00-pv.ini not in scanned files"; exit 1; }

echo "==> Verify 00-pv.ini sets session.save_path under ~/.pv/data/sessions"
SAVE_PATH=$(php -r 'echo ini_get("session.save_path");')
echo "  session.save_path = $SAVE_PATH"
EXPECTED_SAVE_PATH="$HOME/.pv/data/sessions/$PHP_VERSION"
if [ "$SAVE_PATH" != "$EXPECTED_SAVE_PATH" ]; then
    echo "FAIL: session.save_path = $SAVE_PATH, want $EXPECTED_SAVE_PATH"
    exit 1
fi

echo "==> Drop a 99-local.ini and verify it overrides"
echo 'memory_limit = 42M' > "$CONFD_DIR/99-local.ini"
GOT=$(php -r 'echo ini_get("memory_limit");')
if [ "$GOT" != "42M" ]; then
    echo "FAIL: memory_limit = $GOT, want 42M (99-local.ini override didn't apply)"
    rm -f "$CONFD_DIR/99-local.ini"
    exit 1
fi
echo "  OK: 99-local.ini override applied"
rm -f "$CONFD_DIR/99-local.ini"

echo "OK: php-ini phase passed"

# Note: "user edits survive reinstall" and "00-pv.ini regenerated on
# reinstall" are covered by phpenv unit tests in Task 3; replicating them
# here would require re-running the full install which is slow in CI and
# adds little signal beyond the unit tests.
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x scripts/e2e/php-ini.sh
```

- [ ] **Step 3: Wire into the workflow**

In `.github/workflows/e2e.yml`, add a new phase between Phase 3 ("Verify Installation") and Phase 4 ("Test pv env"):

```yaml
      # ── Phase 3.5: PHP ini layout ──────────────────────────────────
      - name: Verify per-version php.ini layout and overrides
        timeout-minutes: 1
        run: scripts/e2e/php-ini.sh
```

(Place it after the existing "Verify both PHP versions installed" step and before "Test pv env PATH setup".)

- [ ] **Step 4: Commit**

```bash
git add scripts/e2e/php-ini.sh .github/workflows/e2e.yml
git commit -m "ci: add e2e phase for per-version php.ini layout"
```

---

## Task 10: Final verification

**Files:** none modified — verification only.

- [ ] **Step 1: Run the full local check**

```bash
gofmt -l . | tee /tmp/gofmt-out
test ! -s /tmp/gofmt-out
go vet ./...
go build ./...
go test ./...
```

Expected: `gofmt -l` outputs nothing, vet clean, build clean, all tests PASS.

- [ ] **Step 2: Confirm imports are alphabetized within groups**

Per CLAUDE.md, `gofmt` does not sort imports; the developer must keep them alphabetical within each group (stdlib, then external) by hand. Open each modified file and visually verify the import block:

- `internal/binaries/download.go` — should have `errors` added in alphabetical position in the stdlib group.
- `internal/phpenv/install.go` — should have `errors` added; existing external imports already in order.
- `internal/phpenv/inilayout.go` — new file: `fmt`, `io`, `os`, `path/filepath` (stdlib), then `github.com/prvious/pv/internal/config`.
- `internal/server/frankenphp.go` — should have `github.com/prvious/pv/internal/phpenv` added in alphabetical position in the external group.
- `internal/server/frankenphp_test.go` — new file: `path/filepath`, `strings`, `testing` (stdlib), then `github.com/prvious/pv/internal/config`.

- [ ] **Step 3: Self-test the install path on the local machine (smoke)**

```bash
go build -o pv .
./pv php:install 8.4    # may already be installed; backfill should still run cleanly
ls -la ~/.pv/php/8.4/etc/ ~/.pv/php/8.4/conf.d/
~/.pv/bin/php --ri core | head -20
~/.pv/bin/php -r 'echo "ini=" . php_ini_loaded_file() . PHP_EOL . "scanned=" . php_ini_scanned_files() . PHP_EOL;'
```

Expected:
- `~/.pv/php/8.4/etc/php.ini`, `php.ini-development`, and `~/.pv/php/8.4/conf.d/00-pv.ini` all present.
- `Configuration File (php.ini) Path => ~/.pv/php/8.4/etc`.
- `Loaded Configuration File => ~/.pv/php/8.4/etc/php.ini`.
- Scanned files include `00-pv.ini`.

If any of these are off, debug before pushing.

- [ ] **Step 4: Push the branch**

```bash
git push -u origin <branch-name>
```

- [ ] **Step 5: Dispatch the scoped CI build (from Task 8)**

If not yet done in Task 8:

```bash
gh workflow run build-artifacts.yml --ref <branch-name> \
  -f skip_postgres=true -f skip_mysql=true
gh run watch
```

Expected: all three FrankenPHP matrix jobs pass; the new sanity check (tarball extract verification) passes.

- [ ] **Step 6: Open the PR**

```bash
gh pr create --title "feat: per-version php.ini for PHP CLI and FrankenPHP" --body "$(cat <<'EOF'
## Summary
- Each installed PHP version gets `~/.pv/php/<ver>/etc/php.ini` (upstream `php.ini-development` verbatim) and `conf.d/` with a pv-managed `00-pv.ini` of path defaults.
- The `php` shim and FrankenPHP launcher both export `PHPRC` and `PHP_INI_SCAN_DIR`, so CLI and server load the same ini for a given version.
- `php.ini-development` is bundled in the existing `php-mac-*.tar.gz` artifact; old artifacts without it are tolerated via `binaries.ErrEntryNotFound`.

## Test plan
- [ ] `go test ./...` passes
- [ ] `gofmt -l .` is empty, `go vet ./...` clean
- [ ] CI: build-artifacts dispatch (FrankenPHP only) green
- [ ] CI: e2e workflow green, including the new php-ini phase
- [ ] Manual: `~/.pv/bin/php -r 'echo php_ini_loaded_file();'` reports the per-version path
- [ ] Manual: dropping `99-local.ini` overrides `00-pv.ini` directives
- [ ] Manual: user edits to `php.ini` survive `pv php:update`

Spec: `docs/superpowers/specs/2026-05-04-per-version-php-ini-design.md`
EOF
)"
```

---

## Self-Review Notes

Spec coverage check (each spec section → task):

- **Filesystem layout** → Task 3 (creates dirs), Task 4 (extracts ini-development).
- **Where the upstream template comes from** → Task 8 (build-artifacts).
- **Install / update / uninstall lifecycle** → Tasks 3, 4. Uninstall is unchanged (already removes the version dir).
- **Backfill** → Task 4 (EnsureInstalled branch) and Task 7 (daemon start).
- **`00-pv.ini` content** → Task 3 (`writePvIni`).
- **Wiring the env vars** → Task 5 (shim) and Task 6 (FrankenPHP).
- **Components and contracts** table → all helpers exist as named (`PhpEtcDir`, `PhpConfDDir`, `PhpEnv`, `EnsureIniLayout`, `phpenv.Install/InstallProgress` extension, `tools.writePhpShim` extension, `server.StartFrankenPHP` + `StartVersionFrankenPHP`).
- **Build-artifacts.yml change** → Task 8.
- **Testing** unit + e2e → Tasks 1, 2, 3, 5, 6, 9.
- **Risks and edge cases** → all addressed: existing php.ini preserved (Task 3 test), 00-pv.ini regenerated (Task 3 test), missing ini-development source tolerated (Task 3 + 4), older artifacts tolerated (Task 2 sentinel), global PHP unset gracefully handled (Task 6 — `phpenv.GlobalVersion()` returns error, ignored, version passed empty).

No placeholders, no "implement later", no "similar to Task N". Function names are consistent across tasks: `EnsureIniLayout`, `frankenphpEnv`, `PhpEnv`, `PhpEtcDir`, `PhpConfDDir`, `PhpSessionDir`, `PhpTmpDir`, `ErrEntryNotFound`.
