# Mailpit Native Binary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Docker-based mail service (`axllent/mailpit` image) with the upstream Mailpit binary, supervised by the pv daemon using the infrastructure built by the rustfs plan.

**Architecture:** Purely additive on top of the rustfs plan. Add a new `Mailpit` struct implementing the existing `BinaryService` interface, a new `binaries.Mailpit` descriptor, and an `installMailpit` function. No interface changes, no supervisor changes, no command logic changes. Delete the old Docker `Mail` struct.

**Tech Stack:** Go, existing `binaries` package, existing `services.BinaryService` interface, existing `supervisor` + `ServerManager` infrastructure from the rustfs plan.

**Spec:** `docs/superpowers/specs/2026-04-14-mailpit-native-binary-design.md`

**Dependency:** `docs/superpowers/plans/2026-04-14-rustfs-native-binary.md` must be fully merged before starting this plan. This plan assumes:
- `services.BinaryService`, `services.ReadyCheck`, `services.LookupBinary`, `services.AllBinary` exist
- `internal/supervisor/` package exists
- `ServerManager.reconcileBinaryServices` exists and is called from `Reconcile()`
- `buildSupervisorProcess` / `writeDaemonStatus` / `ReadDaemonStatus` exist in `internal/server/`
- `resolveKind` and all binary-service command paths exist
- `ServiceInstance.Kind` / `ServiceInstance.Enabled` fields exist on the registry

---

## File Structure

| Path | Action | Responsibility |
|------|--------|---------------|
| `internal/binaries/mailpit.go` | Create | Platform-specific archive name + download URL for Mailpit |
| `internal/binaries/mailpit_test.go` | Create | URL construction tests |
| `internal/binaries/manager.go` | Modify | Add `mailpit` cases in `DownloadURL` and `LatestVersionURL` |
| `internal/binaries/install.go` | Modify | Add `installMailpit` function; add `"mailpit":` case to `InstallBinary` switch |
| `internal/services/mailpit.go` | Create | `Mailpit` struct implementing `BinaryService`, registered as `"mail"` |
| `internal/services/mailpit_test.go` | Create | Method-output tests (EnvVars pinned as golden map) |
| `internal/services/mail.go` | Delete | Old Docker `Mail` struct replaced by `Mailpit` |
| `internal/services/mail_test.go` | Delete | Tests for deleted struct |
| `internal/services/service.go` | Modify | Remove `"mail"` from Docker `registry` map |
| `scripts/e2e/mail-binary.sh` | Create | E2E lifecycle test |
| `.github/workflows/e2e.yml` | Modify | Add mail-binary phase |

---

## Task 1: Verify Mailpit distribution

**Files:**
- None modified. Research-only.

Just like the rustfs Task 1 — confirm assumptions before coding.

- [ ] **Step 1: Confirm GitHub API asset list (expected to match the spec)**

```bash
curl -s https://api.github.com/repos/axllent/mailpit/releases/latest | \
  python3 -c "import json,sys; d=json.load(sys.stdin); print('tag:', d['tag_name']); [print(' -', a['name']) for a in d['assets']]"
```

Expected: tag like `v1.29.x`, and assets including:
- `mailpit-darwin-arm64.tar.gz`
- `mailpit-darwin-amd64.tar.gz`
- `mailpit-linux-arm64.tar.gz`
- `mailpit-linux-amd64.tar.gz`

If any of those four filenames is missing or different, stop and amend `docs/superpowers/specs/2026-04-14-mailpit-native-binary-design.md` before proceeding.

- [ ] **Step 2: Download + extract + inspect**

```bash
cd /tmp
TAG=$(curl -s https://api.github.com/repos/axllent/mailpit/releases/latest | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')
curl -L -o mailpit-test.tgz "https://github.com/axllent/mailpit/releases/download/${TAG}/mailpit-darwin-arm64.tar.gz"
mkdir -p /tmp/mailpit-extract
tar -xzf mailpit-test.tgz -C /tmp/mailpit-extract
ls /tmp/mailpit-extract
```

Record:
- Is `mailpit` at the root of the tarball or nested?
- Are there any sibling files (LICENSE, README, etc.)?

- [ ] **Step 3: Verify flag names**

```bash
chmod +x /tmp/mailpit-extract/mailpit
/tmp/mailpit-extract/mailpit --help 2>&1 | head -80
```

Confirm:
- `--smtp <addr>` accepts `:1025`
- `--listen <addr>` accepts `:8025`
- `--database <path>` accepts a file path (not a directory)

If flag names differ (e.g. `--db` vs `--database`), amend the spec's `Mailpit.Args()` specification and the Task 3 implementation code below before proceeding.

- [ ] **Step 4: Verify `/livez` endpoint exists**

```bash
/tmp/mailpit-extract/mailpit --smtp :11025 --listen :18025 --database /tmp/mailpit-test.db &
MAILPIT_PID=$!
sleep 1
curl -v http://127.0.0.1:18025/livez
kill $MAILPIT_PID
rm -f /tmp/mailpit-test.db
```

Expected: HTTP 200 with a small JSON/text body. If `/livez` returns 404, swap the `HTTPEndpoint` in `Mailpit.ReadyCheck()` to `/api/v1/info` (always present) or fall back to a TCP probe.

- [ ] **Step 5: Check binary linkage (optional)**

```bash
otool -L /tmp/mailpit-extract/mailpit  # macOS
# or: ldd /tmp/mailpit-extract/mailpit  # Linux
```

Mailpit is a pure-Go binary and should have zero dynamic dependencies outside libSystem on macOS. If `otool -L` shows user-land libraries we don't control, flag in the spec.

---

## Task 2: `binaries.Mailpit` descriptor + `installMailpit`

**Files:**
- Create: `internal/binaries/mailpit.go`
- Create: `internal/binaries/mailpit_test.go`
- Modify: `internal/binaries/manager.go`
- Modify: `internal/binaries/install.go`

- [ ] **Step 1: Write the failing tests**

Create `internal/binaries/mailpit_test.go`:

```go
package binaries

import (
	"runtime"
	"strings"
	"testing"
)

func TestMailpitURL_CurrentPlatform(t *testing.T) {
	url, err := mailpitURL("v1.29.6")
	if err != nil {
		t.Fatalf("unexpected error for %s/%s: %v", runtime.GOOS, runtime.GOARCH, err)
	}
	if !strings.HasPrefix(url, "https://github.com/axllent/mailpit/releases/download/v1.29.6/") {
		t.Errorf("URL = %q; missing expected prefix", url)
	}
	if !strings.HasSuffix(url, ".tar.gz") {
		t.Errorf("URL = %q; expected .tar.gz suffix", url)
	}
}

func TestMailpitArchiveName_AllPlatforms(t *testing.T) {
	tests := []struct {
		goos, goarch, want string
	}{
		{"darwin", "arm64", "mailpit-darwin-arm64.tar.gz"},
		{"darwin", "amd64", "mailpit-darwin-amd64.tar.gz"},
		{"linux", "amd64", "mailpit-linux-amd64.tar.gz"},
		{"linux", "arm64", "mailpit-linux-arm64.tar.gz"},
	}
	for _, tc := range tests {
		archMap, ok := mailpitPlatformNames[tc.goos]
		if !ok {
			t.Errorf("no entry for GOOS=%s", tc.goos)
			continue
		}
		platform, ok := archMap[tc.goarch]
		if !ok {
			t.Errorf("no entry for GOARCH=%s on %s", tc.goarch, tc.goos)
			continue
		}
		got := "mailpit-" + platform + ".tar.gz"
		if got != tc.want {
			t.Errorf("%s/%s: got %q, want %q", tc.goos, tc.goarch, got, tc.want)
		}
	}
}

func TestDownloadURL_MailpitCase(t *testing.T) {
	url, err := DownloadURL(Mailpit, "v1.29.6")
	if err != nil {
		t.Fatalf("DownloadURL returned error: %v", err)
	}
	if url == "" {
		t.Error("DownloadURL returned empty string")
	}
}

func TestLatestVersionURL_MailpitCase(t *testing.T) {
	got := LatestVersionURL(Mailpit)
	want := "https://api.github.com/repos/axllent/mailpit/releases/latest"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/binaries/ -run Mailpit -v
```

Expected: FAIL — `undefined: mailpitURL`, `undefined: mailpitPlatformNames`, `undefined: Mailpit`.

- [ ] **Step 3: Create the descriptor file**

Create `internal/binaries/mailpit.go`:

```go
package binaries

import (
	"fmt"
	"runtime"
)

var Mailpit = Binary{
	Name:         "mailpit",
	DisplayName:  "Mailpit",
	NeedsExtract: true,
}

var mailpitPlatformNames = map[string]map[string]string{
	"darwin": {
		"arm64": "darwin-arm64",
		"amd64": "darwin-amd64",
	},
	"linux": {
		"amd64": "linux-amd64",
		"arm64": "linux-arm64",
	},
}

func mailpitArchiveName() (string, error) {
	archMap, ok := mailpitPlatformNames[runtime.GOOS]
	if !ok {
		return "", fmt.Errorf("unsupported OS for Mailpit: %s", runtime.GOOS)
	}
	platform, ok := archMap[runtime.GOARCH]
	if !ok {
		return "", fmt.Errorf("unsupported architecture for Mailpit: %s/%s", runtime.GOOS, runtime.GOARCH)
	}
	return fmt.Sprintf("mailpit-%s.tar.gz", platform), nil
}

func mailpitURL(version string) (string, error) {
	archive, err := mailpitArchiveName()
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://github.com/axllent/mailpit/releases/download/%s/%s", version, archive), nil
}
```

- [ ] **Step 4: Wire into `manager.go`**

Edit `internal/binaries/manager.go`.

In `DownloadURL`, before `default:`:

```go
case "mailpit":
    return mailpitURL(version)
```

In `LatestVersionURL`, before `default:`:

```go
case "mailpit":
    return "https://api.github.com/repos/axllent/mailpit/releases/latest"
```

Do **not** add `Mailpit` to `Tools()` — backing service, not user-facing tool.

- [ ] **Step 5: Add `installMailpit` and wire into `InstallBinary`**

Edit `internal/binaries/install.go`. Add a new function, mirroring `installMago`:

```go
func installMailpit(client *http.Client, url string, progress ProgressFunc) error {
	internalBin := config.InternalBinDir()
	archivePath := filepath.Join(internalBin, "mailpit.tar.gz")
	destPath := filepath.Join(internalBin, "mailpit")

	if err := DownloadProgress(client, url, archivePath, progress); err != nil {
		return err
	}
	if err := ExtractTarGz(archivePath, destPath, "mailpit"); err != nil {
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
case "mailpit":
	return installMailpit(client, url, progress)
default:
	return fmt.Errorf("unknown binary: %s", b.Name)
}
```

- [ ] **Step 6: Run tests**

```bash
gofmt -w internal/binaries/
go vet ./internal/binaries/
go test ./internal/binaries/ -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add internal/binaries/mailpit.go internal/binaries/mailpit_test.go internal/binaries/manager.go internal/binaries/install.go
git commit -m "Add Mailpit binary descriptor and installMailpit

Mirrors the Mago pattern: .tar.gz download + ExtractTarGz + chmod.
Wired into InstallBinary via a new mailpit case. Not added to
Tools() because Mailpit is a backing service, not a CLI tool."
```

---

## Task 3: `Mailpit` service implementation

**Files:**
- Create: `internal/services/mailpit.go`
- Create: `internal/services/mailpit_test.go`

- [ ] **Step 1: Write the failing tests**

Create `internal/services/mailpit_test.go`:

```go
package services

import (
	"reflect"
	"testing"
)

func TestMailpit_RegisteredAsMail(t *testing.T) {
	svc, ok := LookupBinary("mail")
	if !ok {
		t.Fatal("LookupBinary(\"mail\") returned ok=false")
	}
	if _, isMailpit := svc.(*Mailpit); !isMailpit {
		t.Errorf("expected *Mailpit, got %T", svc)
	}
}

func TestMailpit_Name(t *testing.T) {
	m := &Mailpit{}
	if m.Name() != "mail" {
		t.Errorf("Name() = %q, want mail", m.Name())
	}
}

func TestMailpit_Ports(t *testing.T) {
	m := &Mailpit{}
	if m.Port() != 1025 {
		t.Errorf("Port() = %d, want 1025", m.Port())
	}
	if m.ConsolePort() != 8025 {
		t.Errorf("ConsolePort() = %d, want 8025", m.ConsolePort())
	}
}

func TestMailpit_WebRoutes(t *testing.T) {
	m := &Mailpit{}
	want := []WebRoute{
		{Subdomain: "mail", Port: 8025},
	}
	got := m.WebRoutes()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("WebRoutes() = %#v, want %#v", got, want)
	}
}

func TestMailpit_EnvVars_Golden(t *testing.T) {
	// Pinned against the exact keys/values the old Docker Mail service
	// produced so linked projects do not need .env rewrites post-migration.
	m := &Mailpit{}
	got := m.EnvVars("anyproject")
	want := map[string]string{
		"MAIL_MAILER":   "smtp",
		"MAIL_HOST":     "127.0.0.1",
		"MAIL_PORT":     "1025",
		"MAIL_USERNAME": "",
		"MAIL_PASSWORD": "",
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("EnvVars() = %#v, want %#v", got, want)
	}
}

func TestMailpit_Args_UsesDataDir(t *testing.T) {
	m := &Mailpit{}
	args := m.Args("/tmp/mailpit-data")
	// The arg list must mention the provided dataDir somewhere (the flag
	// name might be --database or --data depending on Task 1 verification).
	found := false
	for _, a := range args {
		if a == "/tmp/mailpit-data" || a == "/tmp/mailpit-data/mailpit.db" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("Args() did not include the data dir; got %v", args)
	}
}

func TestMailpit_ReadyCheck_HTTPLivez(t *testing.T) {
	m := &Mailpit{}
	rc := m.ReadyCheck()
	if rc.HTTPEndpoint == "" {
		t.Error("ReadyCheck.HTTPEndpoint must be set (Mailpit uses HTTP probe, not TCP)")
	}
	if rc.TCPPort != 0 {
		t.Errorf("ReadyCheck.TCPPort = %d, want 0", rc.TCPPort)
	}
	if rc.Timeout == 0 {
		t.Error("ReadyCheck.Timeout must be non-zero")
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
go test ./internal/services/ -run Mailpit -v
```

Expected: FAIL — `undefined: Mailpit`.

- [ ] **Step 3: Create the implementation**

Create `internal/services/mailpit.go`:

```go
package services

import (
	"time"

	"github.com/prvious/pv/internal/binaries"
)

type Mailpit struct{}

func (m *Mailpit) Name() string        { return "mail" }
func (m *Mailpit) DisplayName() string { return "Mail (Mailpit)" }

func (m *Mailpit) Binary() binaries.Binary { return binaries.Mailpit }

func (m *Mailpit) Args(dataDir string) []string {
	// Flag names verified in Task 1; adjust here if reality differs.
	return []string{
		"--smtp", ":1025",
		"--listen", ":8025",
		"--database", dataDir + "/mailpit.db",
	}
}

func (m *Mailpit) Env() []string { return nil }

func (m *Mailpit) Port() int        { return 1025 }
func (m *Mailpit) ConsolePort() int { return 8025 }

func (m *Mailpit) WebRoutes() []WebRoute {
	return []WebRoute{
		{Subdomain: "mail", Port: 8025},
	}
}

func (m *Mailpit) EnvVars(_ string) map[string]string {
	return map[string]string{
		"MAIL_MAILER":   "smtp",
		"MAIL_HOST":     "127.0.0.1",
		"MAIL_PORT":     "1025",
		"MAIL_USERNAME": "",
		"MAIL_PASSWORD": "",
	}
}

func (m *Mailpit) ReadyCheck() ReadyCheck {
	return ReadyCheck{
		HTTPEndpoint: "http://127.0.0.1:8025/livez",
		Timeout:      30 * time.Second,
	}
}

func init() {
	binaryRegistry["mail"] = &Mailpit{}
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
gofmt -w internal/services/
go vet ./internal/services/
go test ./internal/services/ -run Mailpit -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/services/mailpit.go internal/services/mailpit_test.go
git commit -m "Add Mailpit service implementation

Mailpit registers itself as \"mail\" in the binary registry. Uses
the HTTP ReadyCheck variant pointing at /livez — the first user of
the HTTP probe since RustFS uses TCP. EnvVars keys/values are pinned
against the current Docker Mail service so linked projects do not
need .env rewrites after the migration."
```

---

## Task 4: Remove the Docker Mail service

**Files:**
- Delete: `internal/services/mail.go`
- Delete: `internal/services/mail_test.go`
- Modify: `internal/services/service.go`

- [ ] **Step 1: Delete the old files**

```bash
git rm internal/services/mail.go internal/services/mail_test.go
```

- [ ] **Step 2: Remove `"mail"` from the Docker `registry`**

Edit `internal/services/service.go`. Remove the `"mail": &Mail{}` entry:

Before:
```go
var registry = map[string]Service{
	"mail":     &Mail{},
	"mysql":    &MySQL{},
	"postgres": &Postgres{},
	"redis":    &Redis{},
}
```

After:
```go
var registry = map[string]Service{
	"mysql":    &MySQL{},
	"postgres": &Postgres{},
	"redis":    &Redis{},
}
```

(`Available()` already returns the union of docker + binary names after the rustfs plan; no change needed there.)

- [ ] **Step 3: Build to catch stale references**

```bash
gofmt -w internal/services/
go vet ./internal/services/
go build ./...
```

If any file still references the deleted `services.Mail` type, fix those references. The most likely offender is anything that imported `services.Mail{}` by name; it should instead use `services.LookupBinary("mail")` or `services.Lookup("mail")` and handle both branches via `resolveKind`. The command dispatcher created by the rustfs plan already handles this correctly.

- [ ] **Step 4: Run full test suite**

```bash
go test ./...
```

Expected: PASS throughout. If a registry test that referenced the `ProjectServices.Mail bool` field fails, remember that field is still valid — it represents "does this project want mail?" — it just now maps to a binary service instead of docker.

- [ ] **Step 5: Commit**

```bash
git add internal/services/service.go
git add -u internal/services/mail.go internal/services/mail_test.go
git commit -m "Remove Docker Mail service; mail is now binary-only

The Mailpit BinaryService registered itself as \"mail\" in Task 3;
the Docker Mail struct is removed along with its tests. The Docker
registry map no longer contains mail. No other code changes needed
because the service:* command dispatcher (from the rustfs plan)
already routes \"mail\" to the binary path via resolveKind."
```

---

## Task 5: E2E test + CI integration

**Files:**
- Create: `scripts/e2e/mail-binary.sh`
- Modify: `.github/workflows/e2e.yml`

- [ ] **Step 1: Write the E2E script**

Create `scripts/e2e/mail-binary.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./helpers.sh
. "$SCRIPT_DIR/helpers.sh"

phase "Mail binary service (Mailpit) lifecycle"

pv start >/dev/null &
START_PID=$!
sleep 3

trap 'kill $START_PID 2>/dev/null || true; pv stop >/dev/null 2>&1 || true' EXIT

step "service:add mail"
pv service:add mail

step "binary exists and is executable"
test -x "$HOME/.pv/internal/bin/mailpit"

step "daemon-status lists mailpit"
test -f "$HOME/.pv/daemon-status.json"
grep -q '"mailpit"' "$HOME/.pv/daemon-status.json"

step "HTTP /livez returns 200"
for i in $(seq 1 20); do
    if curl -fsS http://127.0.0.1:8025/livez >/dev/null 2>&1; then break; fi
    sleep 1
done
curl -fsS http://127.0.0.1:8025/livez >/dev/null

step "SMTP port 1025 is reachable"
nc -z 127.0.0.1 1025

step "service:stop mail"
pv service:stop mail
sleep 2
if curl -fsS http://127.0.0.1:8025/livez >/dev/null 2>&1; then
    echo "FAIL: /livez still answering after service:stop"
    exit 1
fi

step "service:start mail"
pv service:start mail
for i in $(seq 1 20); do
    if curl -fsS http://127.0.0.1:8025/livez >/dev/null 2>&1; then break; fi
    sleep 1
done
curl -fsS http://127.0.0.1:8025/livez >/dev/null

step "service:destroy mail"
pv service:destroy mail
test ! -f "$HOME/.pv/internal/bin/mailpit"
test ! -d "$HOME/.pv/services/mail/latest/data"

step "pv stop"
pv stop || true
trap - EXIT

pass "Mail binary service lifecycle OK"
```

Make executable:

```bash
chmod +x scripts/e2e/mail-binary.sh
```

- [ ] **Step 2: Wire into the CI workflow**

Edit `.github/workflows/e2e.yml`. Add a step after the S3 phase:

```yaml
      - name: E2E — Mail binary service lifecycle
        run: ./scripts/e2e/mail-binary.sh
```

- [ ] **Step 3: Run locally on macOS before pushing**

```bash
go build -o pv .
./scripts/e2e/mail-binary.sh
```

Expected: all steps PASS. On failure: inspect `~/.pv/logs/mailpit.log` for Mailpit's stderr.

- [ ] **Step 4: Commit**

```bash
git add scripts/e2e/mail-binary.sh .github/workflows/e2e.yml
git commit -m "Add E2E phase for Mail (Mailpit) binary service lifecycle

Mirrors the rustfs E2E: service:add, stop, start, destroy. Uses
curl on /livez rather than nc, exercising the HTTP ReadyCheck path
for the first time in CI."
```

---

## Parallelization Guide

Linear. Each task depends on the previous:

- Task 1 (verification) must run first.
- Task 2 (binaries descriptor + installMailpit) must land before Task 3 (service implementation) compiles.
- Task 3 (service implementation) must land before Task 4 (delete Docker Mail) compiles.
- Task 5 (E2E) can run after Task 4 merges.

Total: ~5 small, sequential commits. No parallelism worth coordinating.
