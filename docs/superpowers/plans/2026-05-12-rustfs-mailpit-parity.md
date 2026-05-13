# RustFS Mailpit Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor RustFS and Mailpit to use the same version-shaped lifecycle/state structure as Redis, MySQL, and PostgreSQL.

**Architecture:** RustFS and Mailpit stop using top-level `registry.Services` for install/runtime state. Their internal packages own version-shaped lifecycle helpers backed by `state.json`; command packages own UI, prompts, progress, and daemon signaling. `latest` is the default and only valid version for now, preserving future multi-version signatures without supporting extra versions yet.

**Tech Stack:** Go, Cobra, `internal/state`, `internal/registry`, `internal/supervisor`, `internal/caddy`, native binary installers

---

## File Structure

Create or modify these files:

- `internal/registry/registry.go`: change `ProjectServices.Mail` and `ProjectServices.S3` from bools to strings, add version-specific unbind helpers.
- `internal/registry/registry_test.go`: update boolean expectations and add Mail/S3 version unbind tests.
- `internal/rustfs/version.go`: new default and validation helpers.
- `internal/rustfs/state.go`: new version-shaped wanted state.
- `internal/rustfs/wanted.go`: new wanted-version listing.
- `internal/rustfs/installed.go`: new install detection.
- `internal/rustfs/service.go`: move process builder and route data into parent package, remove `caddy` dependency.
- `internal/rustfs/install.go`: make install pure lifecycle work with `client`, `version`, and progress callback.
- `internal/rustfs/update.go`: make update pure lifecycle work with `client`, `version`, and progress callback.
- `internal/rustfs/uninstall.go`: remove binary/state/version/data and unbind projects, without registry service entries.
- `internal/rustfs/wait.go`: poll TCP port instead of daemon status.
- `internal/rustfs/logs.go`: accept `version` and tail versioned log path.
- `internal/rustfs/templatevars.go`: accept `version`.
- `internal/rustfs/status.go`: delete; commands own status display.
- `internal/rustfs/enable.go`: delete; wanted state replaces enabled flag.
- `internal/rustfs/proc/proc.go`: delete after moving process builder into `internal/rustfs`.
- `internal/mailpit/version.go`: new default and validation helpers.
- `internal/mailpit/state.go`: new version-shaped wanted state.
- `internal/mailpit/wanted.go`: new wanted-version listing.
- `internal/mailpit/installed.go`: new install detection.
- `internal/mailpit/service.go`: move process builder and route data into parent package, remove `caddy` dependency.
- `internal/mailpit/install.go`: make install pure lifecycle work with `client`, `version`, and progress callback.
- `internal/mailpit/update.go`: make update pure lifecycle work with `client`, `version`, and progress callback.
- `internal/mailpit/uninstall.go`: remove binary/state/version/data and unbind projects, without registry service entries.
- `internal/mailpit/wait.go`: poll TCP port instead of daemon status.
- `internal/mailpit/logs.go`: accept `version` and tail versioned log path.
- `internal/mailpit/templatevars.go`: accept `version`.
- `internal/mailpit/status.go`: delete; commands own status display.
- `internal/mailpit/enable.go`: delete; wanted state replaces enabled flag.
- `internal/mailpit/proc/proc.go`: delete after moving process builder into `internal/mailpit`.
- `internal/commands/rustfs/*.go`: move UI/signaling here, add optional `[version]`, update wrappers to take args.
- `internal/commands/mailpit/*.go`: move UI/signaling here, add optional `[version]`, update wrappers to take args.
- `internal/server/manager.go`: reconcile RustFS/Mailpit from wanted versions, not registry services.
- `internal/server/manager_test.go`: update supervisor names and add Mailpit wanted-state reconciliation coverage.
- `internal/caddy/caddy.go`: generate service console routes from installed/wanted state, not registry services.
- `internal/caddy/caddy_test.go`: update route-generation setup.
- `internal/automation/steps/apply_pvyml_services.go`: bind Mailpit/RustFS as `latest` strings and check installed packages directly.
- `internal/automation/steps/apply_pvyml_services_test.go`: update Mail/S3 service binding assertions.
- `cmd/install.go`: call `RunInstall(args []string)` for RustFS/Mailpit.
- `cmd/update.go`: update installed RustFS/Mailpit via command wrappers, not registry services.
- `cmd/uninstall.go`: uninstall RustFS/Mailpit via command wrappers.
- `README.md`: remove stale `internal/services/` source-layout wording if it still describes active binary-service registry behavior.

---

### Task 1: Registry Project Service Shape

**Files:**
- Modify: `internal/registry/registry.go`
- Modify: `internal/registry/registry_test.go`

- [ ] **Step 1: Write failing registry tests for Mail/S3 string bindings**

Add tests near the existing `UnbindRedisVersion`, `UnbindMysqlVersion`, and service-binding tests:

```go
func TestUnbindMailVersion(t *testing.T) {
 r := &Registry{
  Projects: []Project{
   {Name: "a", Services: &ProjectServices{Mail: "latest"}},
   {Name: "b", Services: &ProjectServices{Mail: "future"}},
   {Name: "c", Services: &ProjectServices{Mail: "latest"}},
  },
 }

 r.UnbindMailVersion("latest")

 cases := map[string]string{"a": "", "b": "future", "c": ""}
 for _, p := range r.Projects {
  if got := p.Services.Mail; got != cases[p.Name] {
   t.Errorf("%s: Mail = %q, want %q", p.Name, got, cases[p.Name])
  }
 }
}

func TestUnbindS3Version(t *testing.T) {
 r := &Registry{
  Projects: []Project{
   {Name: "a", Services: &ProjectServices{S3: "latest"}},
   {Name: "b", Services: &ProjectServices{S3: "future"}},
   {Name: "c", Services: &ProjectServices{S3: "latest"}},
  },
 }

 r.UnbindS3Version("latest")

 cases := map[string]string{"a": "", "b": "future", "c": ""}
 for _, p := range r.Projects {
  if got := p.Services.S3; got != cases[p.Name] {
   t.Errorf("%s: S3 = %q, want %q", p.Name, got, cases[p.Name])
  }
 }
}
```

Also update any existing tests that construct `ProjectServices{Mail: true}` or `ProjectServices{S3: true}` to use `"latest"`.

- [ ] **Step 2: Run registry tests and verify failure**

Run: `go test ./internal/registry`

Expected: FAIL because `Mail` and `S3` are still bool fields and the new helpers do not exist.

- [ ] **Step 3: Change `ProjectServices` fields**

In `internal/registry/registry.go`, change the struct to:

```go
type ProjectServices struct {
 Mail     string `json:"mail,omitempty"`
 MySQL    string `json:"mysql,omitempty"`
 Postgres string `json:"postgres,omitempty"`
 Redis    string `json:"redis,omitempty"`
 S3       string `json:"s3,omitempty"`
}
```

Keep the Redis bool-to-string `UnmarshalJSON` compatibility only if existing Redis tests still require it. Do not add Mail/S3 bool compatibility.

- [ ] **Step 4: Update service lookup and unbind logic**

Change `ProjectsUsingService` cases:

```go
case "mail":
 if p.Services.Mail != "" {
  names = append(names, p.Name)
 }
case "s3":
 if p.Services.S3 != "" {
  names = append(names, p.Name)
 }
```

Change `UnbindService` cases:

```go
case "mail":
 r.Projects[i].Services.Mail = ""
case "s3":
 r.Projects[i].Services.S3 = ""
```

Add helpers after `UnbindRedisVersion` or next to the other version-specific helpers:

```go
func (r *Registry) UnbindMailVersion(version string) {
 for i := range r.Projects {
  if r.Projects[i].Services == nil {
   continue
  }
  if r.Projects[i].Services.Mail == version {
   r.Projects[i].Services.Mail = ""
  }
 }
}

func (r *Registry) UnbindS3Version(version string) {
 for i := range r.Projects {
  if r.Projects[i].Services == nil {
   continue
  }
  if r.Projects[i].Services.S3 == version {
   r.Projects[i].Services.S3 = ""
  }
 }
}
```

- [ ] **Step 5: Run registry tests**

Run: `go test ./internal/registry`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add internal/registry/registry.go internal/registry/registry_test.go
git commit -m "refactor(services): make mail and s3 project bindings versioned"
```

---

### Task 2: RustFS Internal Lifecycle Package

**Files:**
- Create: `internal/rustfs/version.go`
- Create: `internal/rustfs/state.go`
- Create: `internal/rustfs/wanted.go`
- Create: `internal/rustfs/installed.go`
- Modify: `internal/rustfs/service.go`
- Modify: `internal/rustfs/install.go`
- Modify: `internal/rustfs/update.go`
- Modify: `internal/rustfs/uninstall.go`
- Modify: `internal/rustfs/wait.go`
- Modify: `internal/rustfs/logs.go`
- Modify: `internal/rustfs/templatevars.go`
- Delete: `internal/rustfs/status.go`
- Delete: `internal/rustfs/enable.go`
- Delete: `internal/rustfs/proc/proc.go`
- Modify tests: `internal/rustfs/lifecycle_test.go`, `internal/rustfs/service_test.go`, `internal/rustfs/templatevars_test.go`

- [ ] **Step 1: Write failing RustFS state tests**

Add to `internal/rustfs/lifecycle_test.go`:

```go
func TestState_SetWantedWantedVersionsRemove(t *testing.T) {
 t.Setenv("HOME", t.TempDir())

 if err := SetWanted(DefaultVersion(), WantedRunning); err != nil {
  t.Fatalf("SetWanted running: %v", err)
 }
 versions, err := WantedVersions()
 if err != nil {
  t.Fatalf("WantedVersions: %v", err)
 }
 if len(versions) != 0 {
  t.Fatalf("WantedVersions should ignore not-installed rustfs, got %v", versions)
 }

 binPath := filepath.Join(config.InternalBinDir(), Binary().Name)
 if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
  t.Fatalf("mkdir bin dir: %v", err)
 }
 if err := os.WriteFile(binPath, []byte("#!/bin/sh\n"), 0o755); err != nil {
  t.Fatalf("write fake binary: %v", err)
 }

 versions, err = WantedVersions()
 if err != nil {
  t.Fatalf("WantedVersions installed: %v", err)
 }
 if len(versions) != 1 || versions[0] != DefaultVersion() {
  t.Fatalf("WantedVersions = %v, want [latest]", versions)
 }

 if err := RemoveVersion(DefaultVersion()); err != nil {
  t.Fatalf("RemoveVersion: %v", err)
 }
 st, err := LoadState()
 if err != nil {
  t.Fatalf("LoadState: %v", err)
 }
 if _, ok := st.Versions[DefaultVersion()]; ok {
  t.Fatalf("state still contains latest after RemoveVersion: %#v", st.Versions)
 }
}

func TestValidateVersion_RejectsNonLatest(t *testing.T) {
 if err := ValidateVersion("1.0.0"); err == nil {
  t.Fatal("expected non-latest rustfs version to fail")
 }
}
```

- [ ] **Step 2: Run RustFS tests and verify failure**

Run: `go test ./internal/rustfs`

Expected: FAIL because `DefaultVersion`, `SetWanted`, `WantedVersions`, and `RemoveVersion` are not implemented in the new shape.

- [ ] **Step 3: Add version helpers**

Create `internal/rustfs/version.go`:

```go
package rustfs

import "fmt"

const defaultVersion = "latest"

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

- [ ] **Step 4: Add state helpers**

Create `internal/rustfs/state.go`:

```go
package rustfs

import (
 "encoding/json"
 "fmt"
 "os"

 "github.com/prvious/pv/internal/state"
)

const stateKey = "rustfs"

const (
 WantedRunning = "running"
 WantedStopped = "stopped"
)

type VersionState struct {
 Wanted string `json:"wanted"`
}

type State struct {
 Versions map[string]VersionState `json:"versions"`
}

func LoadState() (State, error) {
 all, err := state.Load()
 if err != nil {
  return State{Versions: map[string]VersionState{}}, err
 }
 raw, ok := all[stateKey]
 if !ok {
  return State{Versions: map[string]VersionState{}}, nil
 }
 var s State
 if err := json.Unmarshal(raw, &s); err != nil {
  fmt.Fprintf(os.Stderr, "rustfs: state slice corrupt (%v); treating as empty\n", err)
  return State{Versions: map[string]VersionState{}}, nil
 }
 if s.Versions == nil {
  s.Versions = map[string]VersionState{}
 }
 return s, nil
}

func SaveState(s State) error {
 all, err := state.Load()
 if err != nil {
  return err
 }
 if s.Versions == nil {
  s.Versions = map[string]VersionState{}
 }
 payload, err := json.Marshal(s)
 if err != nil {
  return err
 }
 all[stateKey] = payload
 return state.Save(all)
}

func SetWanted(version, wanted string) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 if wanted != WantedRunning && wanted != WantedStopped {
  return fmt.Errorf("rustfs: invalid wanted state %q (want %q or %q)", wanted, WantedRunning, WantedStopped)
 }
 s, err := LoadState()
 if err != nil {
  return err
 }
 s.Versions[version] = VersionState{Wanted: wanted}
 return SaveState(s)
}

func RemoveVersion(version string) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 s, err := LoadState()
 if err != nil {
  return err
 }
 delete(s.Versions, version)
 return SaveState(s)
}
```

- [ ] **Step 5: Add installed and wanted helpers**

Create `internal/rustfs/installed.go`:

```go
package rustfs

import (
 "os"
 "path/filepath"

 "github.com/prvious/pv/internal/config"
)

func BinaryPath(version string) (string, error) {
 if err := ValidateVersion(version); err != nil {
  return "", err
 }
 return filepath.Join(config.InternalBinDir(), Binary().Name), nil
}

func LogPath(version string) (string, error) {
 if err := ValidateVersion(version); err != nil {
  return "", err
 }
 return filepath.Join(config.LogsDir(), Binary().Name+"-"+version+".log"), nil
}

func IsInstalled(version string) bool {
 path, err := BinaryPath(version)
 if err != nil {
  return false
 }
 st, err := os.Stat(path)
 return err == nil && !st.IsDir()
}

func InstalledVersions() ([]string, error) {
 if IsInstalled(DefaultVersion()) {
  return []string{DefaultVersion()}, nil
 }
 return nil, nil
}
```

Create `internal/rustfs/wanted.go`:

```go
package rustfs

func WantedVersions() ([]string, error) {
 st, err := LoadState()
 if err != nil {
  return nil, err
 }
 var versions []string
 for version, entry := range st.Versions {
  if entry.Wanted == WantedRunning && IsInstalled(version) {
   versions = append(versions, version)
  }
 }
 return versions, nil
}
```

- [ ] **Step 6: Move process builder into parent package**

Replace `internal/rustfs/service.go` with the parent-package service data and process builder. Do not import `internal/caddy` or `internal/server` here.

```go
package rustfs

import (
 "fmt"
 "os"
 "path/filepath"
 "time"

 "github.com/prvious/pv/internal/binaries"
 "github.com/prvious/pv/internal/config"
 "github.com/prvious/pv/internal/supervisor"
)

const (
 displayName = "S3 Storage (RustFS)"
 serviceKey  = "s3"
 port        = 9000
 consolePort = 9001
)

type WebRoute struct {
 Subdomain string
 Port      int
}

func Binary() binaries.Binary { return binaries.Rustfs }
func Port() int               { return port }
func ConsolePort() int        { return consolePort }
func DisplayName() string     { return displayName }
func ServiceKey() string      { return serviceKey }

func WebRoutes() []WebRoute {
 return []WebRoute{
  {Subdomain: "s3", Port: consolePort},
  {Subdomain: "s3-api", Port: port},
 }
}

func EnvVars(version, projectName string) map[string]string {
 if err := ValidateVersion(version); err != nil {
  return map[string]string{}
 }
 return map[string]string{
  "AWS_ACCESS_KEY_ID":           "rstfsadmin",
  "AWS_SECRET_ACCESS_KEY":       "rstfsadmin",
  "AWS_DEFAULT_REGION":          "us-east-1",
  "AWS_BUCKET":                  projectName,
  "AWS_ENDPOINT":                "http://127.0.0.1:9000",
  "AWS_USE_PATH_STYLE_ENDPOINT": "true",
 }
}

func BuildSupervisorProcess(version string) (supervisor.Process, error) {
 if err := ValidateVersion(version); err != nil {
  return supervisor.Process{}, err
 }
 binPath, err := BinaryPath(version)
 if err != nil {
  return supervisor.Process{}, err
 }
 dataDir := config.ServiceDataDir(serviceKey, version)
 if err := os.MkdirAll(dataDir, 0o755); err != nil {
  return supervisor.Process{}, fmt.Errorf("create data dir %s: %w", dataDir, err)
 }
 logFile, err := LogPath(version)
 if err != nil {
  return supervisor.Process{}, err
 }
 if err := os.MkdirAll(filepath.Dir(logFile), 0o755); err != nil {
  return supervisor.Process{}, fmt.Errorf("create log dir: %w", err)
 }
 rc := supervisor.TCPReady(port, 30*time.Second)
 ready, err := supervisor.BuildReadyFunc(rc)
 if err != nil {
  return supervisor.Process{}, fmt.Errorf("rustfs: %w", err)
 }
 args := []string{
  "server", dataDir,
  "--address", fmt.Sprintf(":%d", port),
  "--console-enable",
  "--console-address", fmt.Sprintf(":%d", consolePort),
 }
 env := []string{
  "RUSTFS_ACCESS_KEY=rstfsadmin",
  "RUSTFS_SECRET_KEY=rstfsadmin",
 }
 return supervisor.Process{
  Name:         Binary().Name + "-" + version,
  Binary:       binPath,
  Args:         args,
  Env:          env,
  LogFile:      logFile,
  Ready:        ready,
  ReadyTimeout: rc.Timeout,
 }, nil
}
```

- [ ] **Step 7: Rewrite install/update/uninstall helpers**

In `internal/rustfs/install.go`, remove `registry`, `server`, `caddy`, `ui`, and stderr printing. Keep only lifecycle work:

```go
func Install(client *http.Client, version string) error {
 return InstallProgress(client, version, nil)
}

func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 if err := config.EnsureDirs(); err != nil {
  return err
 }
 latest, err := binaries.FetchLatestVersion(client, Binary())
 if err != nil {
  return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
 }
 if err := binaries.InstallBinaryProgress(client, Binary(), latest, progress); err != nil {
  return err
 }
 vs, err := binaries.LoadVersions()
 if err != nil {
  return fmt.Errorf("cannot load versions state: %w", err)
 }
 vs.Set(Binary().Name, latest)
 if err := vs.Save(); err != nil {
  return fmt.Errorf("cannot save versions state: %w", err)
 }
 if err := os.MkdirAll(config.ServiceDataDir(ServiceKey(), version), 0o755); err != nil {
  return fmt.Errorf("create rustfs data dir: %w", err)
 }
 return SetWanted(version, WantedRunning)
}
```

In `internal/rustfs/update.go`:

```go
func Update(client *http.Client, version string) error {
 return UpdateProgress(client, version, nil)
}

func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 if !IsInstalled(version) {
  return fmt.Errorf("rustfs %s is not installed", version)
 }
 latest, err := binaries.FetchLatestVersion(client, Binary())
 if err != nil {
  return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
 }
 if err := binaries.InstallBinaryProgress(client, Binary(), latest, progress); err != nil {
  return err
 }
 vs, err := binaries.LoadVersions()
 if err != nil {
  return fmt.Errorf("cannot load versions state: %w", err)
 }
 vs.Set(Binary().Name, latest)
 return vs.Save()
}
```

In `internal/rustfs/uninstall.go`, keep filesystem/state cleanup and project unbinding, but remove registry service removal, caddy generation, daemon signaling, and UI:

```go
func Uninstall(version string, force bool) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 _ = SetWanted(version, WantedStopped)
 _ = WaitStopped(version, 30*time.Second)

 binPath, err := BinaryPath(version)
 if err != nil {
  return err
 }
 if err := os.Remove(binPath); err != nil && !os.IsNotExist(err) {
  return fmt.Errorf("remove rustfs binary: %w", err)
 }
 if logPath, err := LogPath(version); err == nil {
  _ = os.Remove(logPath)
 }
 if err := RemoveVersion(version); err != nil {
  return err
 }
 if vs, err := binaries.LoadVersions(); err == nil {
  delete(vs.Versions, Binary().Name)
  _ = vs.Save()
 }
 if force {
  if err := os.RemoveAll(config.ServiceDataDir(ServiceKey(), version)); err != nil {
   return fmt.Errorf("cannot delete data: %w", err)
  }
 }
 reg, err := registry.Load()
 if err != nil {
  return err
 }
 reg.UnbindS3Version(version)
 return reg.Save()
}
```

- [ ] **Step 8: Remove server imports from wait/status paths**

Replace `internal/rustfs/wait.go` with TCP polling:

```go
package rustfs

import (
 "fmt"
 "net"
 "time"
)

func WaitStopped(version string, timeout time.Duration) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 addr := fmt.Sprintf("127.0.0.1:%d", Port())
 deadline := time.Now().Add(timeout)
 for time.Now().Before(deadline) {
  c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
  if err != nil {
   return nil
  }
  c.Close()
  time.Sleep(200 * time.Millisecond)
 }
 return fmt.Errorf("rustfs %s did not stop within %s", version, timeout)
}
```

Delete `internal/rustfs/status.go`; command status will read daemon status.

- [ ] **Step 9: Version logs and template vars**

Change `TailLog` signature in `internal/rustfs/logs.go`:

```go
func TailLog(ctx context.Context, version string, follow bool) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 logPath, err := LogPath(version)
 if err != nil {
  return err
 }
 f, err := os.Open(logPath)
 if err != nil {
  if os.IsNotExist(err) {
   return fmt.Errorf("no log file yet (%s). Has the service run?", logPath)
  }
  return err
 }
 defer f.Close()

 if _, err := io.Copy(os.Stdout, f); err != nil {
  return err
 }
 if !follow {
  return nil
 }
 for {
  select {
  case <-ctx.Done():
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

Change `TemplateVars` signature in `internal/rustfs/templatevars.go`:

```go
func TemplateVars(version string) map[string]string {
 if err := ValidateVersion(version); err != nil {
  return map[string]string{}
 }
 return map[string]string{
  "endpoint":       fmt.Sprintf("http://127.0.0.1:%d", Port()),
  "access_key":     "rstfsadmin",
  "secret_key":     "rstfsadmin",
  "region":         "us-east-1",
  "use_path_style": "true",
 }
}
```

- [ ] **Step 10: Delete obsolete RustFS files**

Remove:

```bash
rm internal/rustfs/status.go
rm internal/rustfs/enable.go
rm internal/rustfs/proc/proc.go
```

If `internal/rustfs/proc/` becomes empty, remove the directory with normal filesystem cleanup outside git tracking.

- [ ] **Step 11: Run RustFS package tests**

Run: `go test ./internal/rustfs`

Expected: PASS after updating tests for string project bindings, versioned `TemplateVars`, process name `rustfs-latest`, and versioned log path.

- [ ] **Step 12: Commit**

```bash
git add internal/rustfs
git commit -m "refactor(rustfs): use versioned wanted state"
```

---

### Task 3: Mailpit Internal Lifecycle Package

**Files:**
- Create: `internal/mailpit/version.go`
- Create: `internal/mailpit/state.go`
- Create: `internal/mailpit/wanted.go`
- Create: `internal/mailpit/installed.go`
- Modify: `internal/mailpit/service.go`
- Modify: `internal/mailpit/install.go`
- Modify: `internal/mailpit/update.go`
- Modify: `internal/mailpit/uninstall.go`
- Modify: `internal/mailpit/wait.go`
- Modify: `internal/mailpit/logs.go`
- Modify: `internal/mailpit/templatevars.go`
- Delete: `internal/mailpit/status.go`
- Delete: `internal/mailpit/enable.go`
- Delete: `internal/mailpit/proc/proc.go`
- Modify tests: `internal/mailpit/lifecycle_test.go`, `internal/mailpit/service_test.go`, `internal/mailpit/templatevars_test.go`

- [ ] **Step 1: Write failing Mailpit state tests**

Add to `internal/mailpit/lifecycle_test.go`:

```go
func TestState_SetWantedWantedVersionsRemove(t *testing.T) {
 t.Setenv("HOME", t.TempDir())

 if err := SetWanted(DefaultVersion(), WantedRunning); err != nil {
  t.Fatalf("SetWanted running: %v", err)
 }
 versions, err := WantedVersions()
 if err != nil {
  t.Fatalf("WantedVersions: %v", err)
 }
 if len(versions) != 0 {
  t.Fatalf("WantedVersions should ignore not-installed mailpit, got %v", versions)
 }

 binPath := filepath.Join(config.InternalBinDir(), Binary().Name)
 if err := os.MkdirAll(filepath.Dir(binPath), 0o755); err != nil {
  t.Fatalf("mkdir bin dir: %v", err)
 }
 if err := os.WriteFile(binPath, []byte("#!/bin/sh\n"), 0o755); err != nil {
  t.Fatalf("write fake binary: %v", err)
 }

 versions, err = WantedVersions()
 if err != nil {
  t.Fatalf("WantedVersions installed: %v", err)
 }
 if len(versions) != 1 || versions[0] != DefaultVersion() {
  t.Fatalf("WantedVersions = %v, want [latest]", versions)
 }

 if err := RemoveVersion(DefaultVersion()); err != nil {
  t.Fatalf("RemoveVersion: %v", err)
 }
 st, err := LoadState()
 if err != nil {
  t.Fatalf("LoadState: %v", err)
 }
 if _, ok := st.Versions[DefaultVersion()]; ok {
  t.Fatalf("state still contains latest after RemoveVersion: %#v", st.Versions)
 }
}

func TestValidateVersion_RejectsNonLatest(t *testing.T) {
 if err := ValidateVersion("1.0.0"); err == nil {
  t.Fatal("expected non-latest mailpit version to fail")
 }
}
```

- [ ] **Step 2: Run Mailpit tests and verify failure**

Run: `go test ./internal/mailpit`

Expected: FAIL because the new version/state helpers do not exist.

- [ ] **Step 3: Add version helpers**

Create `internal/mailpit/version.go`:

```go
package mailpit

import "fmt"

const defaultVersion = "latest"

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

- [ ] **Step 4: Add state, installed, and wanted helpers**

Create `internal/mailpit/state.go`:

```go
package mailpit

import (
 "encoding/json"
 "fmt"
 "os"

 "github.com/prvious/pv/internal/state"
)

const stateKey = "mailpit"

const (
 WantedRunning = "running"
 WantedStopped = "stopped"
)

type VersionState struct {
 Wanted string `json:"wanted"`
}

type State struct {
 Versions map[string]VersionState `json:"versions"`
}

func LoadState() (State, error) {
 all, err := state.Load()
 if err != nil {
  return State{Versions: map[string]VersionState{}}, err
 }
 raw, ok := all[stateKey]
 if !ok {
  return State{Versions: map[string]VersionState{}}, nil
 }
 var s State
 if err := json.Unmarshal(raw, &s); err != nil {
  fmt.Fprintf(os.Stderr, "mailpit: state slice corrupt (%v); treating as empty\n", err)
  return State{Versions: map[string]VersionState{}}, nil
 }
 if s.Versions == nil {
  s.Versions = map[string]VersionState{}
 }
 return s, nil
}

func SaveState(s State) error {
 all, err := state.Load()
 if err != nil {
  return err
 }
 if s.Versions == nil {
  s.Versions = map[string]VersionState{}
 }
 payload, err := json.Marshal(s)
 if err != nil {
  return err
 }
 all[stateKey] = payload
 return state.Save(all)
}

func SetWanted(version, wanted string) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 if wanted != WantedRunning && wanted != WantedStopped {
  return fmt.Errorf("mailpit: invalid wanted state %q (want %q or %q)", wanted, WantedRunning, WantedStopped)
 }
 s, err := LoadState()
 if err != nil {
  return err
 }
 s.Versions[version] = VersionState{Wanted: wanted}
 return SaveState(s)
}

func RemoveVersion(version string) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 s, err := LoadState()
 if err != nil {
  return err
 }
 delete(s.Versions, version)
 return SaveState(s)
}
```

Create `internal/mailpit/installed.go`:

```go
package mailpit

import (
 "os"
 "path/filepath"

 "github.com/prvious/pv/internal/config"
)

func BinaryPath(version string) (string, error) {
 if err := ValidateVersion(version); err != nil {
  return "", err
 }
 return filepath.Join(config.InternalBinDir(), Binary().Name), nil
}

func LogPath(version string) (string, error) {
 if err := ValidateVersion(version); err != nil {
  return "", err
 }
 return filepath.Join(config.LogsDir(), Binary().Name+"-"+version+".log"), nil
}

func IsInstalled(version string) bool {
 path, err := BinaryPath(version)
 if err != nil {
  return false
 }
 st, err := os.Stat(path)
 return err == nil && !st.IsDir()
}

func InstalledVersions() ([]string, error) {
 if IsInstalled(DefaultVersion()) {
  return []string{DefaultVersion()}, nil
 }
 return nil, nil
}
```

Create `internal/mailpit/wanted.go`:

```go
package mailpit

func WantedVersions() ([]string, error) {
 st, err := LoadState()
 if err != nil {
  return nil, err
 }
 var versions []string
 for version, entry := range st.Versions {
  if entry.Wanted == WantedRunning && IsInstalled(version) {
   versions = append(versions, version)
  }
 }
 return versions, nil
}
```

- [ ] **Step 5: Move process builder into parent package**

Replace `internal/mailpit/service.go` with:

```go
package mailpit

import (
 "fmt"
 "os"
 "path/filepath"
 "time"

 "github.com/prvious/pv/internal/binaries"
 "github.com/prvious/pv/internal/config"
 "github.com/prvious/pv/internal/supervisor"
)

const (
 displayName = "Mail (Mailpit)"
 serviceKey  = "mail"
 port        = 1025
 consolePort = 8025
)

type WebRoute struct {
 Subdomain string
 Port      int
}

func Binary() binaries.Binary { return binaries.Mailpit }
func Port() int               { return port }
func ConsolePort() int        { return consolePort }
func DisplayName() string     { return displayName }
func ServiceKey() string      { return serviceKey }

func WebRoutes() []WebRoute {
 return []WebRoute{{Subdomain: "mail", Port: consolePort}}
}

func EnvVars(version, _ string) map[string]string {
 if err := ValidateVersion(version); err != nil {
  return map[string]string{}
 }
 return map[string]string{
  "MAIL_MAILER":   "smtp",
  "MAIL_HOST":     "127.0.0.1",
  "MAIL_PORT":     "1025",
  "MAIL_USERNAME": "",
  "MAIL_PASSWORD": "",
 }
}

func BuildSupervisorProcess(version string) (supervisor.Process, error) {
 if err := ValidateVersion(version); err != nil {
  return supervisor.Process{}, err
 }
 binPath, err := BinaryPath(version)
 if err != nil {
  return supervisor.Process{}, err
 }
 dataDir := config.ServiceDataDir(serviceKey, version)
 if err := os.MkdirAll(dataDir, 0o755); err != nil {
  return supervisor.Process{}, fmt.Errorf("create data dir %s: %w", dataDir, err)
 }
 logFile, err := LogPath(version)
 if err != nil {
  return supervisor.Process{}, err
 }
 if err := os.MkdirAll(filepath.Dir(logFile), 0o755); err != nil {
  return supervisor.Process{}, fmt.Errorf("create log dir: %w", err)
 }
 rc := supervisor.HTTPReady(fmt.Sprintf("http://127.0.0.1:%d/livez", consolePort), 30*time.Second)
 ready, err := supervisor.BuildReadyFunc(rc)
 if err != nil {
  return supervisor.Process{}, fmt.Errorf("mailpit: %w", err)
 }
 args := []string{
  "--smtp", fmt.Sprintf(":%d", port),
  "--listen", fmt.Sprintf(":%d", consolePort),
  "--database", dataDir + "/mailpit.db",
 }
 return supervisor.Process{
  Name:         Binary().Name + "-" + version,
  Binary:       binPath,
  Args:         args,
  Env:          nil,
  LogFile:      logFile,
  Ready:        ready,
  ReadyTimeout: rc.Timeout,
 }, nil
}
```

- [ ] **Step 6: Rewrite install/update/uninstall helpers**

In `internal/mailpit/install.go`, remove `registry`, `server`, `caddy`, `ui`, and stderr printing. Keep only lifecycle work:

```go
func Install(client *http.Client, version string) error {
 return InstallProgress(client, version, nil)
}

func InstallProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 if err := config.EnsureDirs(); err != nil {
  return err
 }
 latest, err := binaries.FetchLatestVersion(client, Binary())
 if err != nil {
  return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
 }
 if err := binaries.InstallBinaryProgress(client, Binary(), latest, progress); err != nil {
  return err
 }
 vs, err := binaries.LoadVersions()
 if err != nil {
  return fmt.Errorf("cannot load versions state: %w", err)
 }
 vs.Set(Binary().Name, latest)
 if err := vs.Save(); err != nil {
  return fmt.Errorf("cannot save versions state: %w", err)
 }
 if err := os.MkdirAll(config.ServiceDataDir(ServiceKey(), version), 0o755); err != nil {
  return fmt.Errorf("create mailpit data dir: %w", err)
 }
 return SetWanted(version, WantedRunning)
}
```

In `internal/mailpit/update.go`:

```go
func Update(client *http.Client, version string) error {
 return UpdateProgress(client, version, nil)
}

func UpdateProgress(client *http.Client, version string, progress binaries.ProgressFunc) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 if !IsInstalled(version) {
  return fmt.Errorf("mailpit %s is not installed", version)
 }
 latest, err := binaries.FetchLatestVersion(client, Binary())
 if err != nil {
  return fmt.Errorf("cannot resolve latest %s version: %w", Binary().DisplayName, err)
 }
 if err := binaries.InstallBinaryProgress(client, Binary(), latest, progress); err != nil {
  return err
 }
 vs, err := binaries.LoadVersions()
 if err != nil {
  return fmt.Errorf("cannot load versions state: %w", err)
 }
 vs.Set(Binary().Name, latest)
 return vs.Save()
}
```

In `internal/mailpit/uninstall.go`:

```go
func Uninstall(version string, force bool) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 _ = SetWanted(version, WantedStopped)
 _ = WaitStopped(version, 30*time.Second)

 binPath, err := BinaryPath(version)
 if err != nil {
  return err
 }
 if err := os.Remove(binPath); err != nil && !os.IsNotExist(err) {
  return fmt.Errorf("remove mailpit binary: %w", err)
 }
 if logPath, err := LogPath(version); err == nil {
  _ = os.Remove(logPath)
 }
 if err := RemoveVersion(version); err != nil {
  return err
 }
 if vs, err := binaries.LoadVersions(); err == nil {
  delete(vs.Versions, Binary().Name)
  _ = vs.Save()
 }
 if force {
  if err := os.RemoveAll(config.ServiceDataDir(ServiceKey(), version)); err != nil {
   return fmt.Errorf("cannot delete data: %w", err)
  }
 }
 reg, err := registry.Load()
 if err != nil {
  return err
 }
 reg.UnbindMailVersion(version)
 return reg.Save()
}
```

- [ ] **Step 7: Remove server imports from wait/status paths**

Replace `internal/mailpit/wait.go` with TCP polling against `Port()`:

```go
package mailpit

import (
 "fmt"
 "net"
 "time"
)

func WaitStopped(version string, timeout time.Duration) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 addr := fmt.Sprintf("127.0.0.1:%d", Port())
 deadline := time.Now().Add(timeout)
 for time.Now().Before(deadline) {
  c, err := net.DialTimeout("tcp", addr, 200*time.Millisecond)
  if err != nil {
   return nil
  }
  c.Close()
  time.Sleep(200 * time.Millisecond)
 }
 return fmt.Errorf("mailpit %s did not stop within %s", version, timeout)
}
```

Delete `internal/mailpit/status.go`.

- [ ] **Step 8: Version logs and template vars**

Change `TailLog` signature in `internal/mailpit/logs.go`:

```go
func TailLog(ctx context.Context, version string, follow bool) error {
 if err := ValidateVersion(version); err != nil {
  return err
 }
 logPath, err := LogPath(version)
 if err != nil {
  return err
 }
 f, err := os.Open(logPath)
 if err != nil {
  if os.IsNotExist(err) {
   return fmt.Errorf("no log file yet (%s). Has the service run?", logPath)
  }
  return err
 }
 defer f.Close()

 if _, err := io.Copy(os.Stdout, f); err != nil {
  return err
 }
 if !follow {
  return nil
 }
 for {
  select {
  case <-ctx.Done():
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

Change `TemplateVars` signature in `internal/mailpit/templatevars.go`:

```go
func TemplateVars(version string) map[string]string {
 if err := ValidateVersion(version); err != nil {
  return map[string]string{}
 }
 return map[string]string{
  "smtp_host": "127.0.0.1",
  "smtp_port": strconv.Itoa(Port()),
  "http_host": "127.0.0.1",
  "http_port": strconv.Itoa(ConsolePort()),
 }
}
```

- [ ] **Step 9: Delete obsolete Mailpit files**

Remove:

```bash
rm internal/mailpit/status.go
rm internal/mailpit/enable.go
rm internal/mailpit/proc/proc.go
```

- [ ] **Step 10: Run Mailpit package tests**

Run: `go test ./internal/mailpit`

Expected: PASS after updating tests for string project bindings, versioned `TemplateVars`, process name `mailpit-latest`, and versioned log path.

- [ ] **Step 11: Commit**

```bash
git add internal/mailpit
git commit -m "refactor(mailpit): use versioned wanted state"
```

---

### Task 4: Command Packages Own UI and Daemon Signaling

**Files:**
- Modify: `internal/commands/rustfs/register.go`
- Modify: `internal/commands/rustfs/install.go`
- Modify: `internal/commands/rustfs/update.go`
- Modify: `internal/commands/rustfs/uninstall.go`
- Modify: `internal/commands/rustfs/start.go`
- Modify: `internal/commands/rustfs/stop.go`
- Modify: `internal/commands/rustfs/restart.go`
- Modify: `internal/commands/rustfs/status.go`
- Modify: `internal/commands/rustfs/logs.go`
- Modify: `internal/commands/mailpit/register.go`
- Modify: `internal/commands/mailpit/install.go`
- Modify: `internal/commands/mailpit/update.go`
- Modify: `internal/commands/mailpit/uninstall.go`
- Modify: `internal/commands/mailpit/start.go`
- Modify: `internal/commands/mailpit/stop.go`
- Modify: `internal/commands/mailpit/restart.go`
- Modify: `internal/commands/mailpit/status.go`
- Modify: `internal/commands/mailpit/logs.go`
- Modify tests: `internal/commands/rustfs/register_test.go`, `internal/commands/mailpit/register_test.go`

- [ ] **Step 1: Update command registration wrappers**

In both `register.go` files, change wrappers to accept args and add force uninstall:

```go
func RunInstall(args []string) error {
 return installCmd.RunE(installCmd, args)
}

func RunUpdate(args []string) error {
 return updateCmd.RunE(updateCmd, args)
}

func RunUninstall(args []string) error {
 return uninstallCmd.RunE(uninstallCmd, args)
}

func UninstallForce(version string) error {
 prev := uninstallForce
 uninstallForce = true
 defer func() { uninstallForce = prev }()
 return uninstallCmd.RunE(uninstallCmd, []string{version})
}
```

- [ ] **Step 2: Add command-local daemon signal helper**

Create `internal/commands/rustfs/helpers.go` and `internal/commands/mailpit/helpers.go` with this package-local helper:

```go
func signalDaemon(serviceName string) error {
 if !server.IsRunning() {
  ui.Subtle(fmt.Sprintf("daemon not running - %s will start on next `pv start`", serviceName))
  return nil
 }
 return server.SignalDaemon()
}
```

Use ASCII hyphen in the message.

- [ ] **Step 3: Refactor install commands**

RustFS `install.go` should accept optional version and own progress/UI:

```go
Use:  "rustfs:install [version]",
Args: cobra.MaximumNArgs(1),
RunE: func(cmd *cobra.Command, args []string) error {
 version := ""
 if len(args) > 0 {
  version = args[0]
 }
 resolved, err := pkg.ResolveVersion(version)
 if err != nil {
  return err
 }
 client := &http.Client{Timeout: 5 * time.Minute}
 if pkg.IsInstalled(resolved) {
  if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
   return err
  }
  ui.Success(fmt.Sprintf("%s %s already installed - marked as wanted running.", pkg.DisplayName(), resolved))
  return signalDaemon(pkg.DisplayName())
 }
 if err := ui.StepProgress(fmt.Sprintf("Installing %s %s...", pkg.DisplayName(), resolved), func(progress func(written, total int64)) (string, error) {
  if err := pkg.InstallProgress(client, resolved, progress); err != nil {
   return "", err
  }
  return fmt.Sprintf("Installed %s %s", pkg.DisplayName(), resolved), nil
 }); err != nil {
  return err
 }
 if err := caddy.GenerateServiceSiteConfigs(nil); err != nil {
  ui.Subtle(fmt.Sprintf("Could not generate service site config: %v", err))
 }
 if err := signalDaemon(pkg.DisplayName()); err != nil {
  return err
 }
 printConnectionDetails(resolved)
 return nil
},
```

Mailpit uses the same flow with `mailpit` package identifiers and text.

- [ ] **Step 4: Refactor start and stop commands**

Use optional `[version]`, resolve it, set wanted state, and signal daemon.

Start command body:

```go
resolved, err := pkg.ResolveVersion(argVersion(args))
if err != nil {
 return err
}
if !pkg.IsInstalled(resolved) {
 ui.Subtle(fmt.Sprintf("%s %s is not installed (run `pv %s:install %s`).", pkg.DisplayName(), resolved, pkg.Binary().Name, resolved))
 return nil
}
if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
 return err
}
ui.Success(fmt.Sprintf("%s %s marked running.", pkg.DisplayName(), resolved))
return signalDaemon(pkg.DisplayName())
```

Stop command body:

```go
resolved, err := pkg.ResolveVersion(argVersion(args))
if err != nil {
 return err
}
if err := pkg.SetWanted(resolved, pkg.WantedStopped); err != nil {
 return err
}
ui.Success(fmt.Sprintf("%s %s marked stopped.", pkg.DisplayName(), resolved))
if server.IsRunning() {
 return server.SignalDaemon()
}
return nil
```

Add helper in each command package:

```go
func argVersion(args []string) string {
 if len(args) > 0 {
  return args[0]
 }
 return ""
}
```

- [ ] **Step 5: Refactor restart commands**

Command body:

```go
resolved, err := pkg.ResolveVersion(argVersion(args))
if err != nil {
 return err
}
if err := pkg.SetWanted(resolved, pkg.WantedStopped); err != nil {
 return err
}
if server.IsRunning() {
 if err := server.SignalDaemon(); err != nil {
  return fmt.Errorf("signal daemon: %w", err)
 }
 if err := pkg.WaitStopped(resolved, 30*time.Second); err != nil {
  return err
 }
}
if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
 return err
}
ui.Success(fmt.Sprintf("%s %s restarted.", pkg.DisplayName(), resolved))
return signalDaemon(pkg.DisplayName())
```

- [ ] **Step 6: Refactor update commands**

Command body:

```go
resolved, err := pkg.ResolveVersion(argVersion(args))
if err != nil {
 return err
}
if !pkg.IsInstalled(resolved) {
 return fmt.Errorf("%s %s is not installed", pkg.Binary().Name, resolved)
}
wasRunning := false
if st, err := pkg.LoadState(); err == nil {
 if entry, ok := st.Versions[resolved]; ok && entry.Wanted == pkg.WantedRunning {
  wasRunning = true
 }
}
if err := pkg.SetWanted(resolved, pkg.WantedStopped); err != nil {
 return err
}
if server.IsRunning() {
 if err := server.SignalDaemon(); err != nil {
  return fmt.Errorf("signal daemon: %w", err)
 }
 if err := pkg.WaitStopped(resolved, 30*time.Second); err != nil {
  return err
 }
}
client := &http.Client{Timeout: 5 * time.Minute}
if err := ui.StepProgress(fmt.Sprintf("Updating %s %s...", pkg.DisplayName(), resolved), func(progress func(written, total int64)) (string, error) {
 if err := pkg.UpdateProgress(client, resolved, progress); err != nil {
  return "", err
 }
 return fmt.Sprintf("Updated %s %s", pkg.DisplayName(), resolved), nil
}); err != nil {
 return err
}
if wasRunning {
 if err := pkg.SetWanted(resolved, pkg.WantedRunning); err != nil {
  return err
 }
}
ui.Success(fmt.Sprintf("%s %s updated.", pkg.DisplayName(), resolved))
if server.IsRunning() {
 return server.SignalDaemon()
}
return nil
```

- [ ] **Step 7: Refactor uninstall commands**

Use optional `[version]`; prompt unless `--force`; command applies fallback before package uninstall if fallback needs project bindings.

Command body shape:

```go
resolved, err := pkg.ResolveVersion(argVersion(args))
if err != nil {
 return err
}
if !pkg.IsInstalled(resolved) {
 ui.Subtle(fmt.Sprintf("%s %s is not installed.", pkg.DisplayName(), resolved))
 return nil
}
if !uninstallForce {
 confirmed := false
 if err := huh.NewConfirm().
  Title(fmt.Sprintf("Remove %s %s and DELETE its data directory? This cannot be undone.", pkg.DisplayName(), resolved)).
  Affirmative("Yes").
  Negative("No").
  Value(&confirmed).
  Run(); err != nil {
  return err
 }
 if !confirmed {
  return fmt.Errorf("aborted")
 }
}
if err := pkg.SetWanted(resolved, pkg.WantedStopped); err != nil {
 return err
}
if server.IsRunning() {
 if err := server.SignalDaemon(); err != nil {
  return fmt.Errorf("signal daemon: %w", err)
 }
 if err := pkg.WaitStopped(resolved, 30*time.Second); err != nil {
  return err
 }
}
reg, err := registry.Load()
if err != nil {
 return err
}
pkg.ApplyFallbacksToLinkedProjects(reg)
if err := pkg.Uninstall(resolved, true); err != nil {
 return err
}
if err := caddy.GenerateServiceSiteConfigs(nil); err != nil {
 ui.Subtle(fmt.Sprintf("Could not regenerate service site config: %v", err))
}
ui.Success(fmt.Sprintf("%s %s uninstalled.", pkg.DisplayName(), resolved))
return nil
```

- [ ] **Step 8: Refactor status and logs commands**

Status command body:

```go
resolved, err := pkg.ResolveVersion(argVersion(args))
if err != nil {
 return err
}
if !pkg.IsInstalled(resolved) {
 ui.Subtle(fmt.Sprintf("%s %s is not installed.", pkg.DisplayName(), resolved))
 return nil
}
status, _ := server.ReadDaemonStatus()
key := pkg.Binary().Name + "-" + resolved
if status != nil {
 if s, ok := status.Supervised[key]; ok && s.Running {
  ui.Success(fmt.Sprintf("%s: running (pid %d)", key, s.PID))
  return nil
 }
}
ui.Subtle(fmt.Sprintf("%s: stopped", key))
return nil
```

Logs command body:

```go
resolved, err := pkg.ResolveVersion(argVersion(args))
if err != nil {
 return err
}
return pkg.TailLog(cmd.Context(), resolved, logsFollow)
```

- [ ] **Step 9: Run command package tests**

Run: `go test ./internal/commands/rustfs ./internal/commands/mailpit`

Expected: PASS after updating canonical command `Use` strings in tests if needed. Alias tests must still pass.

- [ ] **Step 10: Commit**

```bash
git add internal/commands/rustfs internal/commands/mailpit
git commit -m "refactor(services): move rustfs and mailpit orchestration to commands"
```

---

### Task 5: Server Reconcile and Caddy Routes Without Registry Services

**Files:**
- Modify: `internal/server/manager.go`
- Modify: `internal/server/manager_test.go`
- Modify: `internal/caddy/caddy.go`
- Modify: `internal/caddy/caddy_test.go`

- [ ] **Step 1: Update server imports**

In `internal/server/manager.go`, replace proc imports:

```go
"github.com/prvious/pv/internal/mailpit"
"github.com/prvious/pv/internal/rustfs"
```

Remove:

```go
mailpitproc "github.com/prvious/pv/internal/mailpit/proc"
rustfsproc "github.com/prvious/pv/internal/rustfs/proc"
```

- [ ] **Step 2: Reconcile RustFS/Mailpit from wanted versions**

Replace the Source 1 registry blocks with:

```go
// Source 1a - rustfs, singleton version-shaped service.
rustfsVersions, rustfsErr := rustfs.WantedVersions()
if rustfsErr != nil {
 startErrors = append(startErrors, fmt.Sprintf("rustfs: wanted: %v", rustfsErr))
}
for _, version := range rustfsVersions {
 proc, err := rustfs.BuildSupervisorProcess(version)
 if err != nil {
  startErrors = append(startErrors, fmt.Sprintf("rustfs-%s: build: %v", version, err))
 } else {
  wanted["rustfs-"+version] = proc
 }
}

// Source 1b - mailpit, singleton version-shaped service.
mailpitVersions, mailpitErr := mailpit.WantedVersions()
if mailpitErr != nil {
 startErrors = append(startErrors, fmt.Sprintf("mailpit: wanted: %v", mailpitErr))
}
for _, version := range mailpitVersions {
 proc, err := mailpit.BuildSupervisorProcess(version)
 if err != nil {
  startErrors = append(startErrors, fmt.Sprintf("mailpit-%s: build: %v", version, err))
 } else {
  wanted["mailpit-"+version] = proc
 }
}
```

Add transient-error guards in the stop loop:

```go
if rustfsErr != nil && strings.HasPrefix(supKey, "rustfs-") {
 continue
}
if mailpitErr != nil && strings.HasPrefix(supKey, "mailpit-") {
 continue
}
```

- [ ] **Step 3: Update server tests**

In `internal/server/manager_test.go`, update RustFS expected supervisor name from `rustfs` to `rustfs-latest`. Set wanted state with:

```go
if err := rustfs.SetWanted(rustfs.DefaultVersion(), rustfs.WantedRunning); err != nil {
 t.Fatalf("SetWanted rustfs: %v", err)
}
```

Add a Mailpit reconcile test with fake binary, wanted-running state, and expected `mailpit-latest` running. Use the existing fake RustFS helper pattern from the file; write the fake executable to `filepath.Join(config.InternalBinDir(), "mailpit")`.

- [ ] **Step 4: Update Caddy imports and route generation**

In `internal/caddy/caddy.go`, replace proc imports with parent package imports:

```go
"github.com/prvious/pv/internal/mailpit"
"github.com/prvious/pv/internal/rustfs"
```

Change `GenerateServiceSiteConfigs` so it ignores `registry.Services` and builds routes from installed/wanted state:

```go
func GenerateServiceSiteConfigs(reg *registry.Registry) error {
 settings, err := config.LoadSettings()
 if err != nil {
  return err
 }
 tmpl, err := template.New("serviceConsole").Parse(serviceConsoleTmpl)
 if err != nil {
  return err
 }

 routeSets := [][]WebRoute{}
 if rustfsRoutesEnabled() {
  var routes []WebRoute
  for _, r := range rustfs.WebRoutes() {
   routes = append(routes, WebRoute{Subdomain: r.Subdomain, Port: r.Port})
  }
  routeSets = append(routeSets, routes)
 }
 if mailpitRoutesEnabled() {
  var routes []WebRoute
  for _, r := range mailpit.WebRoutes() {
   routes = append(routes, WebRoute{Subdomain: r.Subdomain, Port: r.Port})
  }
  routeSets = append(routeSets, routes)
 }

 for _, routes := range routeSets {
  for _, route := range routes {
   var buf bytes.Buffer
   if err := tmpl.Execute(&buf, serviceConsoleData{Subdomain: route.Subdomain, TLD: settings.Defaults.TLD, Port: route.Port}); err != nil {
    return err
   }
   outPath := filepath.Join(config.SitesDir(), "_svc-"+route.Subdomain+".caddy")
   if err := os.WriteFile(outPath, buf.Bytes(), 0o644); err != nil {
    return err
   }
  }
 }
 return nil
}
```

Add helpers in the same file:

```go
func rustfsRoutesEnabled() bool {
 if rustfs.IsInstalled(rustfs.DefaultVersion()) {
  return true
 }
 versions, err := rustfs.WantedVersions()
 if err != nil {
  return false
 }
 return slices.Contains(versions, rustfs.DefaultVersion())
}

func mailpitRoutesEnabled() bool {
 if mailpit.IsInstalled(mailpit.DefaultVersion()) {
  return true
 }
 versions, err := mailpit.WantedVersions()
 if err != nil {
  return false
 }
 return slices.Contains(versions, mailpit.DefaultVersion())
}
```

Add `slices` to imports.

- [ ] **Step 5: Update Caddy tests**

In `internal/caddy/caddy_test.go`, replace route-generation setup that populates `reg.Services` with fake installed binaries:

```go
binDir := config.InternalBinDir()
if err := os.MkdirAll(binDir, 0o755); err != nil {
 t.Fatalf("mkdir bin dir: %v", err)
}
if err := os.WriteFile(filepath.Join(binDir, "rustfs"), []byte("#!/bin/sh\n"), 0o755); err != nil {
 t.Fatalf("write rustfs: %v", err)
}
if err := os.WriteFile(filepath.Join(binDir, "mailpit"), []byte("#!/bin/sh\n"), 0o755); err != nil {
 t.Fatalf("write mailpit: %v", err)
}
```

Use an empty registry:

```go
reg := &registry.Registry{Services: map[string]*registry.ServiceInstance{}}
```

- [ ] **Step 6: Run server and Caddy tests**

Run: `go test ./internal/server ./internal/caddy`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add internal/server internal/caddy
git commit -m "refactor(services): reconcile mailpit and rustfs from wanted state"
```

---

### Task 6: Automation, Orchestrators, and Docs

**Files:**
- Modify: `internal/automation/steps/apply_pvyml_services.go`
- Modify: `internal/automation/steps/apply_pvyml_services_test.go`
- Modify: `internal/config/pvyml.go`
- Modify: `cmd/install.go`
- Modify: `cmd/update.go`
- Modify: `cmd/uninstall.go`
- Modify: `README.md`

- [ ] **Step 1: Update pv.yml service binding logic**

In `apply_pvyml_services.go`, add imports:

```go
"github.com/prvious/pv/internal/mailpit"
"github.com/prvious/pv/internal/rustfs"
```

Replace Mailpit block:

```go
if cfg.Mailpit != nil {
 version, err := mailpit.ResolveVersion(cfg.Mailpit.Version)
 if err != nil {
  return "", err
 }
 if !mailpit.IsInstalled(version) {
  return "", fmt.Errorf("pv.yml mailpit %s is not installed - run `pv mailpit:install %s`", version, version)
 }
 bindProjectMail(ctx.Registry, ctx.ProjectName, version)
 count++
}
```

Replace RustFS block:

```go
if cfg.Rustfs != nil {
 version, err := rustfs.ResolveVersion(cfg.Rustfs.Version)
 if err != nil {
  return "", err
 }
 if !rustfs.IsInstalled(version) {
  return "", fmt.Errorf("pv.yml rustfs %s is not installed - run `pv rustfs:install %s`", version, version)
 }
 bindProjectS3(ctx.Registry, ctx.ProjectName, version)
 count++
}
```

Remove `findServiceByName` and `bindProjectService`. Add:

```go
func bindProjectMail(reg *registry.Registry, projectName, version string) {
 for i := range reg.Projects {
  if reg.Projects[i].Name != projectName {
   continue
  }
  if reg.Projects[i].Services == nil {
   reg.Projects[i].Services = &registry.ProjectServices{}
  }
  reg.Projects[i].Services.Mail = version
  return
 }
}

func bindProjectS3(reg *registry.Registry, projectName, version string) {
 for i := range reg.Projects {
  if reg.Projects[i].Name != projectName {
   continue
  }
  if reg.Projects[i].Services == nil {
   reg.Projects[i].Services = &registry.ProjectServices{}
  }
  reg.Projects[i].Services.S3 = version
  return
 }
}
```

- [ ] **Step 2: Update pv.yml config comment**

In `internal/config/pvyml.go`, update the `ServiceConfig` comment:

```go
// Version is required for postgresql and mysql. Redis, mailpit, and rustfs
// default to their package default versions when omitted; mailpit and rustfs
// currently accept only "latest".
```

- [ ] **Step 3: Update automation tests**

In `apply_pvyml_services_test.go`, replace assertions for booleans with string values:

```go
if reg.Projects[0].Services == nil || reg.Projects[0].Services.Mail != mailpit.DefaultVersion() {
 t.Fatalf("Mail = %q, want %q", reg.Projects[0].Services.Mail, mailpit.DefaultVersion())
}
```

And:

```go
if reg.Projects[0].Services == nil || reg.Projects[0].Services.S3 != rustfs.DefaultVersion() {
 t.Fatalf("S3 = %q, want %q", reg.Projects[0].Services.S3, rustfs.DefaultVersion())
}
```

Set up fake installed binaries for Mailpit/RustFS tests by writing executable files to `config.InternalBinDir()`.

- [ ] **Step 4: Update `cmd/install.go` wrappers**

Change `installBinaryService` cases:

```go
case "s3":
 return rustfscmd.RunInstall(nil)
case "mail":
 return mailpitcmd.RunInstall(nil)
```

- [ ] **Step 5: Update `cmd/update.go`**

Remove direct imports of `internal/mailpit`, `internal/rustfs`, and binary-service registry update logic. Add command imports if missing:

```go
mailpitCmds "github.com/prvious/pv/internal/commands/mailpit"
rustfsCmds "github.com/prvious/pv/internal/commands/rustfs"
"github.com/prvious/pv/internal/mailpit"
"github.com/prvious/pv/internal/rustfs"
```

Replace the binary-service update block with installed-version loops:

```go
if versions, err := rustfs.InstalledVersions(); err == nil {
 for _, version := range versions {
  if err := rustfsCmds.RunUpdate([]string{version}); err != nil {
   if !errors.Is(err, ui.ErrAlreadyPrinted) {
    ui.Fail(fmt.Sprintf("RustFS %s update failed: %v", version, err))
   }
   failures = append(failures, "RustFS "+version)
  }
 }
}

if versions, err := mailpit.InstalledVersions(); err == nil {
 for _, version := range versions {
  if err := mailpitCmds.RunUpdate([]string{version}); err != nil {
   if !errors.Is(err, ui.ErrAlreadyPrinted) {
    ui.Fail(fmt.Sprintf("Mailpit %s update failed: %v", version, err))
   }
   failures = append(failures, "Mailpit "+version)
  }
 }
}
```

- [ ] **Step 6: Update `cmd/uninstall.go`**

Add command and package imports:

```go
mailpitCmds "github.com/prvious/pv/internal/commands/mailpit"
rustfsCmds "github.com/prvious/pv/internal/commands/rustfs"
"github.com/prvious/pv/internal/mailpit"
"github.com/prvious/pv/internal/rustfs"
```

Before PHP/Mago/Composer uninstall, add:

```go
if rustfs.IsInstalled(rustfs.DefaultVersion()) {
 if err := rustfsCmds.UninstallForce(rustfs.DefaultVersion()); err != nil {
  hadFailures = true
  if !errors.Is(err, ui.ErrAlreadyPrinted) {
   ui.Fail(fmt.Sprintf("rustfs uninstall failed: %v", err))
  }
 }
}

if mailpit.IsInstalled(mailpit.DefaultVersion()) {
 if err := mailpitCmds.UninstallForce(mailpit.DefaultVersion()); err != nil {
  hadFailures = true
  if !errors.Is(err, ui.ErrAlreadyPrinted) {
   ui.Fail(fmt.Sprintf("mailpit uninstall failed: %v", err))
  }
 }
}
```

- [ ] **Step 7: Update README source layout**

Remove or rewrite the line that says:

```text
services/          # Binary-service registry (mail, s3)
```

Use:

```text
rustfs/, mailpit/  # Native singleton service lifecycle helpers
```

- [ ] **Step 8: Run affected tests**

Run: `go test ./internal/automation/steps ./cmd`

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add internal/automation/steps internal/config/pvyml.go cmd/install.go cmd/update.go cmd/uninstall.go README.md
git commit -m "refactor(services): wire mailpit and rustfs parity through orchestrators"
```

---

### Task 7: Remove Top-Level Registry Service Usage and Verify

**Files:**
- Search all Go files.
- Modify any remaining production references.
- Modify tests that still assume `registry.Services["mail"]` or `registry.Services["s3"]` controls Mailpit/RustFS.

- [ ] **Step 1: Search for forbidden production usage**

Run: `rg 'Services\["(mail|s3)"\]|AddService\("(mail|s3)"|RemoveService\("(mail|s3)"|FindService\("(mail|s3)"' --glob '*.go'`

Expected: No production use for Mailpit/RustFS lifecycle remains. Tests may mention old registry helpers only for registry-generic behavior, not RustFS/Mailpit runtime.

- [ ] **Step 2: Search for deleted proc imports**

Run: `rg 'mailpit/proc|rustfs/proc' --glob '*.go'`

Expected: No matches.

- [ ] **Step 3: Search for boolean Mail/S3 bindings**

Run: `rg 'ProjectServices\{[^}]*((Mail|S3): true|(Mail|S3): false)|Services\.(Mail|S3)\s*=\s*(true|false)' --glob '*.go'`

Expected: No matches.

- [ ] **Step 4: Format**

Run: `gofmt -w .`

Expected: command exits 0.

- [ ] **Step 5: Vet**

Run: `go vet ./...`

Expected: PASS.

- [ ] **Step 6: Build**

Run: `go build ./...`

Expected: PASS.

- [ ] **Step 7: Test**

Run: `go test ./...`

Expected: PASS.

- [ ] **Step 8: Commit final cleanup**

```bash
git add .
git commit -m "test(services): verify mailpit and rustfs parity refactor"
```

---

## Plan Self-Review

- Spec coverage: The plan removes top-level `registry.Services` usage for Mailpit/RustFS, converts project bindings to `latest` strings, adds default-version helpers, moves runtime desired state to `state.json`, moves UI/signaling to command packages, updates daemon reconciliation, updates Caddy routing, and keeps duplication for a future PR.
- Placeholder scan: No placeholder markers or undefined future work is required to execute the tasks. The only deferred work is the explicitly documented future shared abstraction, which is out of scope.
- Type consistency: `DefaultVersion()`, `ResolveVersion`, `ValidateVersion`, `SetWanted`, `WantedVersions`, `BuildSupervisorProcess(version)`, `TailLog(ctx, version, follow)`, `TemplateVars(version)`, `RunInstall(args []string)`, `RunUpdate(args []string)`, and `UninstallForce(version)` are used consistently across RustFS and Mailpit.
