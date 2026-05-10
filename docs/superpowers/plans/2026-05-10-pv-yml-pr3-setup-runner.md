# pv.yml PR 3 — `setup:` runner + db commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the `setup:` command runner — when pv.yml declares a `setup:` block, `pv link` runs the user's shell commands instead of the hardcoded `Composer install / GenerateKey / InstallOctane / CreateDatabase / RunMigrations / CopyEnv` pipeline. Also expose `pv postgres:db:create / :drop` and `pv mysql:db:create / :drop` as standalone commands users can call from `setup:` (the underlying helpers already exist).

**Scope decision:** `pv s3:bucket:create / :drop` is **deferred to a follow-up PR**. The spec assumed bucket-creation logic was already partly present; the explorer confirmed it isn't (rustfs has no bucket code today). Adding it requires either an S3 SDK dependency, inline sigv4, or shelling out to `mc`. That's its own focused PR; the rest of PR 3 stands without it.

**Architecture:**
- `*ProjectConfig.HasSetup()` nil-safe helper, parallel to `HasServices` / `HasAnyEnv` from PR 2.
- `ApplySetupStep` runs each line of `cfg.Setup` via `bash -c`, with the pinned PHP bin dir prepended to PATH so `php artisan ...` finds the right binary. Fail-fast on first non-zero exit. Streams stdout/stderr directly to the user (long commands like `composer install` shouldn't buffer).
- 6 existing pipeline steps gain a `HasSetup()` short-circuit in their `ShouldRun`: `CopyEnvStep`, `ComposerInstallStep`, `GenerateKeyStep`, `InstallOctaneStep`, `CreateDatabaseStep`, `RunMigrationsStep`. When the user owns the pipeline via `setup:`, the legacy steps step out of the way entirely.
- `pv postgres:db:create <name>` / `:drop <name>` and `pv mysql:db:create <name>` / `:drop <name>` are new cobra commands wired through `internal/commands/{postgres,mysql}/register.go`. They reuse `postgres.CreateDatabase` / `mysql.CreateDatabase` (already exported), and add `DropDatabase` siblings if absent.

**Tech Stack:** Go 1.x, cobra, existing `internal/automation/` step framework, `os/exec`.

**Spec reference:** `docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md` (PR 3 section).

**Base commit:** `3bdfaf0` on `main`. Branch: `feat/pvyml-setup-runner`.

---

## Compat strategy

| pv.yml has `setup:`? | Behavior |
|---|---|
| Yes | `ApplySetupStep` runs the user's commands; the 6 legacy steps skip via their `HasSetup()` short-circuit. |
| No | 6 legacy steps run as today (auto-detect path; PR 2's services/env gating still applies). |

Aliases-with-no-setup, services-with-no-setup, env-with-no-setup all keep working. The new gate is independent of the PR 2 gates.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `internal/config/pvyml.go` | Modify | Add `HasSetup() bool` method (nil-safe). |
| `internal/config/pvyml_test.go` | Modify | Table-driven test for `HasSetup`. |
| `internal/postgres/database.go` | Modify | Add `DropDatabase(major, dbName)` if not present. |
| `internal/postgres/database_test.go` | Modify / Create | Test for `DropDatabase`. |
| `internal/mysql/database.go` | Modify | Add `DropDatabase(version, dbName)` if not present. |
| `internal/mysql/database_test.go` | Modify / Create | Test for `DropDatabase`. |
| `internal/commands/postgres/db_create.go` | Create | `pv postgres:db:create <name>` cobra command. |
| `internal/commands/postgres/db_drop.go` | Create | `pv postgres:db:drop <name>`. |
| `internal/commands/postgres/register.go` | Modify | Wire new commands. |
| `internal/commands/mysql/db_create.go` | Create | `pv mysql:db:create <name>`. |
| `internal/commands/mysql/db_drop.go` | Create | `pv mysql:db:drop <name>`. |
| `internal/commands/mysql/register.go` | Modify | Wire new commands. |
| `internal/automation/steps/apply_setup.go` | Create | `ApplySetupStep` — runs `cfg.Setup` lines. |
| `internal/automation/steps/apply_setup_test.go` | Create | Tests: happy path, fail-fast, PHP-on-PATH, ShouldRun gates. |
| `internal/laravel/steps.go` | Modify | Prepend `HasSetup()` short-circuit to 6 `ShouldRun` methods (CopyEnv, ComposerInstall, GenerateKey, InstallOctane, CreateDatabase, RunMigrations). |
| `internal/laravel/steps_test.go` | Modify | Add a skip-test per step (6 tests). |
| `cmd/link.go` | Modify | Insert `&steps.ApplySetupStep{}` into the pipeline at the right position. |

---

## Pipeline order after PR 3

```
InstallPHPStep
CopyEnvStep                  ← gated by HasSetup()
ComposerInstallStep          ← gated by HasSetup()
GenerateKeyStep              ← gated by HasSetup()
InstallOctaneStep            ← gated by HasSetup()
ApplyPvYmlServicesStep
DetectServicesStep           ← gated by HasServices() (PR 2)
laravel.DetectServicesStep   ← gated by HasAnyEnv() (PR 2)
ApplyPvYmlEnvStep
laravel.SetAppURLStep
laravel.SetViteTLSStep
GenerateTLSCertStep
CreateDatabaseStep           ← gated by HasSetup()
RunMigrationsStep            ← gated by HasSetup()
ApplySetupStep               ← new, runs only when HasSetup()
```

`ApplySetupStep` runs **last** so the rest of pv's own state (registry, env writes, certs, Caddy config) is in place before the user's commands execute. Setup commands can rely on `.env` being populated, services being bound, and `pv postgres:db:create` being callable.

---

## Task 1: Add `HasSetup()` helper

**Files:**
- Modify: `internal/config/pvyml.go`
- Modify: `internal/config/pvyml_test.go`

Pure additive, no behavior change yet.

- [ ] **Step 1.1: Failing test**

Append to `internal/config/pvyml_test.go`:

```go
func TestProjectConfig_HasSetup(t *testing.T) {
	tests := []struct {
		name string
		cfg  *ProjectConfig
		want bool
	}{
		{"nil", nil, false},
		{"empty", &ProjectConfig{PHP: "8.4"}, false},
		{"empty slice", &ProjectConfig{Setup: []string{}}, false},
		{"one command", &ProjectConfig{Setup: []string{"composer install"}}, true},
		{"several commands", &ProjectConfig{Setup: []string{"composer install", "php artisan migrate"}}, true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.cfg.HasSetup(); got != tt.want {
				t.Errorf("HasSetup() = %v, want %v", got, tt.want)
			}
		})
	}
}
```

- [ ] **Step 1.2: Verify FAIL**

`go test ./internal/config/ -run TestProjectConfig_HasSetup -v` → FAIL with `cfg.HasSetup undefined`.

- [ ] **Step 1.3: Implement `HasSetup()`**

Append to `internal/config/pvyml.go`:

```go
// HasSetup reports whether pv.yml declares any setup: commands.
// Nil-safe so it can be called on a freshly-loaded *ProjectConfig
// that may not exist for the project. Empty slice (Setup: []) is
// treated as "no setup declared" — same as omitting the block.
func (p *ProjectConfig) HasSetup() bool {
	if p == nil {
		return false
	}
	return len(p.Setup) > 0
}
```

- [ ] **Step 1.4: Verify**

`go test ./internal/config/ -v` → all PASS.

- [ ] **Step 1.5: gofmt + vet + build**

```
gofmt -w internal/config/
go vet ./internal/config/...
go build ./...
```
Expected: clean.

- [ ] **Step 1.6: Commit**

```bash
git add internal/config/pvyml.go internal/config/pvyml_test.go
git commit -m "$(cat <<'EOF'
feat(config): add HasSetup() nil-safe helper on *ProjectConfig

Parallel to HasServices() and HasAnyEnv(). Returns true when
pv.yml declares any setup: command. The PR 3 pipeline gate for
the legacy CopyEnv/ComposerInstall/GenerateKey/InstallOctane/
CreateDatabase/RunMigrations steps uses this — when set, the
user owns the setup pipeline and the legacy steps skip.
EOF
)"
```

---

## Task 2: `postgres.DropDatabase` + `mysql.DropDatabase` + standalone db commands

**Files:**
- Modify: `internal/postgres/database.go` (add `DropDatabase` if absent)
- Modify or Create: `internal/postgres/database_test.go`
- Create: `internal/commands/postgres/db_create.go`
- Create: `internal/commands/postgres/db_drop.go`
- Modify: `internal/commands/postgres/register.go`
- Same for mysql under `internal/mysql/` and `internal/commands/mysql/`

The DB creation helpers already exist (called from `CreateDatabaseStep`). We add `DropDatabase` as a symmetric sibling using the same pattern, then expose four cobra commands.

### Postgres

- [ ] **Step 2.1: Read existing `internal/postgres/database.go`**

```bash
cat internal/postgres/database.go
```

Note the signature of `CreateDatabase` and the underlying mechanism (psql command, libpq via Go SDK, etc.). The `DropDatabase` implementation should mirror it byte-for-byte modulo the SQL statement (`DROP DATABASE IF EXISTS <name>` vs `CREATE DATABASE <name>`).

If `DropDatabase(major, dbName string) error` already exists, skip Step 2.2 and 2.3.

- [ ] **Step 2.2: Failing test for `postgres.DropDatabase`**

If `database_test.go` exists, append. If not, create it with the standard pattern. Use the helper that stages a real postgres binary if the package's existing tests run against real postgres, or a stub if they're mocked. Match whatever `CreateDatabase`'s tests do. For most cases:

```go
func TestDropDatabase_NoOpWhenAbsent(t *testing.T) {
	// Mirror the existing CreateDatabase test's setup (postgres install + start).
	// Then call DropDatabase against a name that doesn't exist — must not error
	// (use DROP DATABASE IF EXISTS semantics).
}
```

(Exact body depends on whether the package tests against a real postgres or stubs `psql`. Match the existing pattern in `database_test.go` if present, or copy the staging helper from `internal/automation/steps/detect_services_test.go` — which uses `config.PostgresBinDir(major)` to stage a binary.)

- [ ] **Step 2.3: Implement `postgres.DropDatabase`**

In `internal/postgres/database.go`, add (mirroring `CreateDatabase`):

```go
// DropDatabase removes the named database from the postgres major
// version, no-op if it doesn't exist.
func DropDatabase(major, dbName string) error {
	// Mirror CreateDatabase: locate psql, exec "DROP DATABASE IF EXISTS <name>"
	// with the same connection params.
}
```

(Exact body is a near-copy of `CreateDatabase` with the SQL statement changed. Read `CreateDatabase` and adapt.)

- [ ] **Step 2.4: Create `pv postgres:db:create <name>` command**

Create `internal/commands/postgres/db_create.go`:

```go
package postgres

import (
	"fmt"

	"github.com/prvious/pv/internal/postgres"
	"github.com/spf13/cobra"
)

var dbCreateCmd = &cobra.Command{
	Use:     "postgres:db:create <name>",
	GroupID: "postgres",
	Short:   "Create a database in the highest-installed PostgreSQL major",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		dbName := args[0]
		majors, err := postgres.InstalledMajors()
		if err != nil {
			return fmt.Errorf("list installed majors: %w", err)
		}
		if len(majors) == 0 {
			return fmt.Errorf("no PostgreSQL installed — run `pv postgres:install <major>`")
		}
		major := majors[len(majors)-1] // highest installed
		if err := postgres.CreateDatabase(major, dbName); err != nil {
			return fmt.Errorf("create %s: %w", dbName, err)
		}
		fmt.Printf("created database %q in postgres %s\n", dbName, major)
		return nil
	},
}
```

(Adjust to match the file's existing command pattern — read `internal/commands/postgres/list.go` for the exact style: imports, comment style, etc.)

- [ ] **Step 2.5: Create `pv postgres:db:drop <name>` command**

Create `internal/commands/postgres/db_drop.go`, parallel to db_create:

```go
package postgres

import (
	"fmt"

	"github.com/prvious/pv/internal/postgres"
	"github.com/spf13/cobra"
)

var dbDropCmd = &cobra.Command{
	Use:     "postgres:db:drop <name>",
	GroupID: "postgres",
	Short:   "Drop a database from the highest-installed PostgreSQL major",
	Args:    cobra.ExactArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		dbName := args[0]
		majors, err := postgres.InstalledMajors()
		if err != nil {
			return fmt.Errorf("list installed majors: %w", err)
		}
		if len(majors) == 0 {
			return fmt.Errorf("no PostgreSQL installed")
		}
		major := majors[len(majors)-1]
		if err := postgres.DropDatabase(major, dbName); err != nil {
			return fmt.Errorf("drop %s: %w", dbName, err)
		}
		fmt.Printf("dropped database %q from postgres %s\n", dbName, major)
		return nil
	},
}
```

- [ ] **Step 2.6: Wire commands into `internal/commands/postgres/register.go`**

Read the existing `register.go`. Find the `cmds := []*cobra.Command{...}` slice. Append `dbCreateCmd` and `dbDropCmd`. The block becomes:

```go
cmds := []*cobra.Command{
    installCmd,
    uninstallCmd,
    updateCmd,
    startCmd,
    stopCmd,
    restartCmd,
    listCmd,
    logsCmd,
    statusCmd,
    downloadCmd,
    dbCreateCmd, // NEW
    dbDropCmd,   // NEW
}
```

The aliasCommand loop (`for _, c := range cmds { parent.AddCommand(c); parent.AddCommand(aliasCommand(c, "postgres:", "pg:")) }`) picks them up automatically — `pv pg:db:create` will work as the alias.

### MySQL

- [ ] **Step 2.7: Repeat steps 2.1–2.6 for mysql**

Same shape, swapping `postgres` → `mysql`, `InstalledMajors` → `InstalledVersions`, and ensuring you read `internal/commands/mysql/register.go` (which per the explorer has no alias namespace — just `parent.AddCommand(c)`, no `aliasCommand` call).

The mysql command files:
- `internal/commands/mysql/db_create.go`
- `internal/commands/mysql/db_drop.go`
- Modify `internal/commands/mysql/register.go` to append both to its `cmds` slice.

For the highest-installed selection, mysql `InstalledVersions()` returns versions like `"8.0", "8.4", "9.7"`. Pick the last entry from a sorted result.

- [ ] **Step 2.8: Verify project-wide**

```
gofmt -w internal/postgres/ internal/mysql/ internal/commands/postgres/ internal/commands/mysql/
go vet ./...
go build ./...
go test ./...
```
Expected: clean. Smoke-test the new commands:

```bash
go build -o /tmp/pv-pr3-smoke .
/tmp/pv-pr3-smoke postgres:db:create --help
/tmp/pv-pr3-smoke postgres:db:drop --help
/tmp/pv-pr3-smoke mysql:db:create --help
/tmp/pv-pr3-smoke mysql:db:drop --help
rm /tmp/pv-pr3-smoke
```
All should print fang-styled help with the `<name>` arg shown.

- [ ] **Step 2.9: Commit**

```bash
git add internal/postgres/database.go internal/postgres/database_test.go \
        internal/mysql/database.go internal/mysql/database_test.go \
        internal/commands/postgres/ internal/commands/mysql/
git commit -m "$(cat <<'EOF'
feat(db): standalone postgres:db:create/:drop and mysql:db:create/:drop

Exposes the existing postgres.CreateDatabase / mysql.CreateDatabase
helpers as standalone cobra commands so users can call them from
pv.yml setup: blocks. Adds DropDatabase siblings (IF EXISTS
semantics) and four new commands wired through the postgres/mysql
command registers. Picks the highest-installed major/version for
the operation. S3 bucket commands are deferred to a follow-up PR
since rustfs has no bucket-creation code today.
EOF
)"
```

(If `DropDatabase` already existed for either engine, drop the file from the `git add` list for that engine — only commit what actually changed.)

---

## Task 3: `ApplySetupStep` — the runner

**Files:**
- Create: `internal/automation/steps/apply_setup.go`
- Create: `internal/automation/steps/apply_setup_test.go`

This step runs every line in `cfg.Setup` via `bash -c` with the pinned PHP bin dir prepended to PATH. Fail-fast on first non-zero exit. Streams stdout/stderr directly (don't buffer — long commands need live output).

PHP path resolution: `phpenv.PHPPath(ctx.PHPVersion)` returns the absolute path to `php` for the pinned version. Its directory is what we prepend to PATH. If `phpenv` exposes a `BinDir(version)` helper, use it; otherwise `filepath.Dir(phpenv.PHPPath(version))`.

- [ ] **Step 3.1: Failing test — happy path runs commands in order, sees pinned PHP**

Create `internal/automation/steps/apply_setup_test.go`:

```go
package steps

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/config"
)

func TestApplySetup_RunsCommandsInOrder(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	projDir := t.TempDir()
	marker := filepath.Join(projDir, "marker")

	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectConfig: &config.ProjectConfig{
			Setup: []string{
				"echo first > " + marker,
				"echo second >> " + marker,
			},
		},
	}
	step := &ApplySetupStep{}
	if !step.ShouldRun(ctx) {
		t.Fatal("ShouldRun: want true")
	}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}

	body, err := os.ReadFile(marker)
	if err != nil {
		t.Fatal(err)
	}
	got := strings.TrimSpace(string(body))
	want := "first\nsecond"
	if got != want {
		t.Errorf("marker = %q, want %q", got, want)
	}
}
```

- [ ] **Step 3.2: Verify FAIL**

`go test ./internal/automation/steps/ -run TestApplySetup_RunsCommandsInOrder -v` → FAIL with `undefined: ApplySetupStep`.

- [ ] **Step 3.3: Implement `ApplySetupStep`**

Create `internal/automation/steps/apply_setup.go`:

```go
package steps

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/prvious/pv/internal/automation"
	"github.com/prvious/pv/internal/phpenv"
)

// ApplySetupStep runs the lines in pv.yml's setup: block via bash -c,
// in order, fail-fast on first non-zero exit. The pinned PHP bin dir
// is prepended to PATH so `php artisan ...` resolves to the project's
// version. Each line gets its own shell — variables don't persist
// across lines. Stdout/stderr stream directly so long commands like
// `composer install` produce live output instead of buffering.
type ApplySetupStep struct{}

var _ automation.Step = (*ApplySetupStep)(nil)

func (s *ApplySetupStep) Label() string  { return "Run pv.yml setup commands" }
func (s *ApplySetupStep) Gate() string   { return "apply_setup" }
func (s *ApplySetupStep) Critical() bool { return true }

func (s *ApplySetupStep) ShouldRun(ctx *automation.Context) bool {
	return ctx.ProjectConfig.HasSetup()
}

func (s *ApplySetupStep) Run(ctx *automation.Context) (string, error) {
	env := buildSetupEnv(ctx.PHPVersion)
	for i, line := range ctx.ProjectConfig.Setup {
		cmd := exec.Command("bash", "-c", line)
		cmd.Dir = ctx.ProjectPath
		cmd.Env = env
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return "", fmt.Errorf("setup[%d] %q: %w", i, line, err)
		}
	}
	return fmt.Sprintf("ran %d command(s)", len(ctx.ProjectConfig.Setup)), nil
}

// buildSetupEnv copies os.Environ() and prepends the pinned PHP's bin
// directory to PATH. If phpVersion is empty (project pinned globally
// or PHP not installed), the host PATH is returned unchanged.
func buildSetupEnv(phpVersion string) []string {
	env := os.Environ()
	if phpVersion == "" {
		return env
	}
	phpBin := phpenv.PHPPath(phpVersion)
	if phpBin == "" {
		return env
	}
	binDir := filepath.Dir(phpBin)
	for i, e := range env {
		if strings.HasPrefix(e, "PATH=") {
			env[i] = "PATH=" + binDir + ":" + strings.TrimPrefix(e, "PATH=")
			return env
		}
	}
	// PATH wasn't set; add it.
	return append(env, "PATH="+binDir)
}
```

(If `phpenv.PHPPath` has a different name in the codebase, adjust. Read `internal/phpenv/` to confirm. The helper may also be called `BinPath`, `PHPBin`, or `Path(version)`.)

- [ ] **Step 3.4: Verify happy path passes**

`go test ./internal/automation/steps/ -run TestApplySetup_RunsCommandsInOrder -v` → PASS.

- [ ] **Step 3.5: Failing tests — fail-fast + ShouldRun edge cases**

Append:

```go
func TestApplySetup_FailFast(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()
	marker := filepath.Join(projDir, "marker")

	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectConfig: &config.ProjectConfig{
			Setup: []string{
				"echo first > " + marker,
				"false", // non-zero exit
				"echo third >> " + marker, // should not run
			},
		},
	}
	step := &ApplySetupStep{}
	_, err := step.Run(ctx)
	if err == nil {
		t.Fatal("Run: want error after `false`, got nil")
	}
	if !strings.Contains(err.Error(), "setup[1]") {
		t.Errorf("err = %v; want it to mention setup[1]", err)
	}

	body, _ := os.ReadFile(marker)
	got := strings.TrimSpace(string(body))
	if got != "first" {
		t.Errorf("marker = %q; want only 'first' (third should not have run)", got)
	}
}

func TestApplySetup_ShouldRunFalseWithoutSetup(t *testing.T) {
	ctx := &automation.Context{
		ProjectConfig: &config.ProjectConfig{PHP: "8.4"},
	}
	step := &ApplySetupStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when no setup declared")
	}
}

func TestApplySetup_ShouldRunFalseWithoutConfig(t *testing.T) {
	ctx := &automation.Context{}
	step := &ApplySetupStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when ProjectConfig is nil")
	}
}

func TestApplySetup_RunsInProjectDir(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	projDir := t.TempDir()
	// Write a marker into the project dir using relative path — proves cwd is correct.
	ctx := &automation.Context{
		ProjectName: "p",
		ProjectPath: projDir,
		ProjectConfig: &config.ProjectConfig{
			Setup: []string{"pwd > pwd-marker"},
		},
	}
	step := &ApplySetupStep{}
	if _, err := step.Run(ctx); err != nil {
		t.Fatalf("Run: %v", err)
	}
	body, err := os.ReadFile(filepath.Join(projDir, "pwd-marker"))
	if err != nil {
		t.Fatal(err)
	}
	resolved, _ := filepath.EvalSymlinks(projDir)
	got := strings.TrimSpace(string(body))
	if got != projDir && got != resolved {
		t.Errorf("pwd = %q; want %q or %q", got, projDir, resolved)
	}
}
```

- [ ] **Step 3.6: Verify**

`go test ./internal/automation/steps/ -run TestApplySetup -v` → all 4 PASS.

- [ ] **Step 3.7: Wire `ApplySetupStep` into the pipeline**

In `cmd/link.go`, find the step list. Append `&steps.ApplySetupStep{}` at the END (after `RunMigrationsStep`). Order:

```go
allSteps := []automation.Step{
    &steps.InstallPHPStep{},
    &steps.CopyEnvStep{},
    &steps.ComposerInstallStep{},
    &steps.GenerateKeyStep{},
    &steps.InstallOctaneStep{},
    &steps.ApplyPvYmlServicesStep{},
    &steps.DetectServicesStep{},
    &laravel.DetectServicesStep{},
    &steps.ApplyPvYmlEnvStep{},
    &laravel.SetAppURLStep{},
    &laravel.SetViteTLSStep{},
    &steps.GenerateTLSCertStep{},
    &steps.CreateDatabaseStep{},
    &steps.RunMigrationsStep{},
    &steps.ApplySetupStep{}, // NEW — last, after pv has finished its own bookkeeping
}
```

Preserve all other entries verbatim.

- [ ] **Step 3.8: Verify project-wide**

```
gofmt -w internal/automation/steps/ cmd/
go vet ./...
go build ./...
go test ./...
```
Expected: clean.

- [ ] **Step 3.9: Commit**

```bash
git add internal/automation/steps/apply_setup.go \
        internal/automation/steps/apply_setup_test.go \
        cmd/link.go
git commit -m "$(cat <<'EOF'
feat(automation): ApplySetupStep — run pv.yml setup: commands

Each line is exec'd via bash -c with the pinned PHP bin dir
prepended to PATH so `php artisan ...` finds the right version.
Stdout/stderr stream directly to the user (no buffering — long
commands like composer install should produce live output).
Fail-fast on first non-zero exit, with error including the
failing line's index. Step runs LAST in the pipeline so pv's
own state (registry, env, certs, Caddy) is in place when the
user's commands execute. Legacy step gating lands in the next
commit.
EOF
)"
```

---

## Task 4: Gate the 6 legacy steps on `HasSetup()`

**Files:**
- Modify: `internal/laravel/steps.go` (6 `ShouldRun` methods)
- Modify: `internal/laravel/steps_test.go` (one new skip test per step = 6 tests)

Each affected step's `ShouldRun` currently returns `isLaravel(ctx.ProjectType) && <step-specific conditions>`. The new gate prepends `if ctx.ProjectConfig.HasSetup() { return false }` so the user's `setup:` block fully replaces the legacy pipeline.

The 6 steps and the lines to modify (from the explorer's earlier mapping):

| Step | Lines |
|---|---|
| `CopyEnvStep` | `internal/laravel/steps.go:21–58` (look for `ShouldRun`) |
| `GenerateKeyStep` | 60–86 |
| `InstallOctaneStep` | 150–187 |
| `ComposerInstallStep` | 189–216 |
| `CreateDatabaseStep` | 282–345 |
| `RunMigrationsStep` | 347–371 |

For each, find the `ShouldRun` method and prepend the same short-circuit:

```go
func (s *<Step>Step) ShouldRun(ctx *automation.Context) bool {
	if ctx.ProjectConfig.HasSetup() {
		return false
	}
	// ... existing conditions preserved verbatim ...
}
```

- [ ] **Step 4.1: Failing test for CopyEnvStep skip**

In `internal/laravel/steps_test.go`, append:

```go
func TestCopyEnvStep_SkipsWhenSetupDeclared(t *testing.T) {
	ctx := &automation.Context{
		ProjectType: "laravel",
		ProjectConfig: &config.ProjectConfig{Setup: []string{"cp .env.example .env"}},
	}
	step := &CopyEnvStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}
```

- [ ] **Step 4.2: Verify FAIL**

`go test ./internal/laravel/ -run TestCopyEnvStep_SkipsWhenSetupDeclared -v` → FAIL.

- [ ] **Step 4.3: Add the short-circuit to `CopyEnvStep.ShouldRun`**

Open `internal/laravel/steps.go`. Find `CopyEnvStep`'s `ShouldRun`. Prepend the short-circuit. The new body:

```go
func (s *CopyEnvStep) ShouldRun(ctx *automation.Context) bool {
	if ctx.ProjectConfig.HasSetup() {
		return false
	}
	// ... existing condition preserved verbatim
}
```

- [ ] **Step 4.4: Verify CopyEnvStep tests pass**

`go test ./internal/laravel/ -run TestCopyEnvStep -v` → all PASS.

- [ ] **Step 4.5–4.8: Repeat 4.1–4.4 for the other 5 steps**

For each of `GenerateKeyStep`, `InstallOctaneStep`, `ComposerInstallStep`, `CreateDatabaseStep`, `RunMigrationsStep`:

1. Append a `Test<Step>_SkipsWhenSetupDeclared` test that builds a Context with `ProjectType: "laravel"` and `Setup: []string{...}`, asserts `ShouldRun` returns false.
2. Run, verify FAIL.
3. Prepend the same short-circuit to that step's `ShouldRun`.
4. Run, verify PASS.

Pattern for each test:

```go
func TestGenerateKeyStep_SkipsWhenSetupDeclared(t *testing.T) {
	ctx := &automation.Context{
		ProjectType: "laravel",
		ProjectConfig: &config.ProjectConfig{Setup: []string{"php artisan key:generate"}},
	}
	step := &GenerateKeyStep{}
	if step.ShouldRun(ctx) {
		t.Errorf("ShouldRun: want false when setup declared")
	}
}
```

(And so on for each step.)

- [ ] **Step 4.9: Verify all 6 skip-tests pass + every existing test still passes**

```
go test ./internal/laravel/ -v
```
Expected: 6 new skip-tests pass; every pre-existing test passes.

- [ ] **Step 4.10: Project-wide check**

```
gofmt -w internal/laravel/
go vet ./...
go build ./...
go test ./...
```
Expected: clean.

- [ ] **Step 4.11: Commit**

```bash
git add internal/laravel/steps.go internal/laravel/steps_test.go
git commit -m "$(cat <<'EOF'
feat(automation): gate 6 legacy steps on HasSetup()

When pv.yml declares setup:, CopyEnvStep, GenerateKeyStep,
InstallOctaneStep, ComposerInstallStep, CreateDatabaseStep, and
RunMigrationsStep all short-circuit their ShouldRun and let the
user-declared setup pipeline run unopposed. Without setup:, each
step keeps its existing condition unchanged — backward compatible
for projects without pv.yml or with pv.yml that doesn't declare
setup.
EOF
)"
```

---

## Task 5: Final verification sweep

**Files:** none modified.

- [ ] **Step 5.1: gofmt + vet + build + test**

```
gofmt -l .
go vet ./...
go test ./...
go build ./...
```
Expected: every gate clean.

- [ ] **Step 5.2: Smoke test the binary**

```bash
go build -o /tmp/pv-pr3-smoke .
/tmp/pv-pr3-smoke --version
/tmp/pv-pr3-smoke postgres:db:create --help
/tmp/pv-pr3-smoke postgres:db:drop --help
/tmp/pv-pr3-smoke mysql:db:create --help
/tmp/pv-pr3-smoke mysql:db:drop --help
/tmp/pv-pr3-smoke link --help
rm /tmp/pv-pr3-smoke
```
Expected: version + 5 help screens render without error.

- [ ] **Step 5.3: Confirm commit count and clean working tree**

```
git status
git log --oneline main..HEAD
```
Expected: working tree clean; 5 commits on the branch (one per Task 1–4 + the plan doc that's already there).

Wait — the plan commit isn't included since we're branching off `feat/pvyml-setup-runner` which already has the plan commit. Actual count: plan commit + 4 implementation commits = 5 commits total on the branch.

---

## Self-Review (already applied)

**Spec coverage:**
- ✅ `setup:` runs commands in order, fail-fast, project root cwd, pinned PHP on PATH — Task 3.
- ✅ Hardcoded pipeline steps skip when setup: present — Task 4.
- ✅ Legacy compat path runs when no setup: — Task 4 (each step preserves existing conditions after the short-circuit).
- ✅ `postgres:db:create / :drop` standalone — Task 2.
- ✅ `mysql:db:create / :drop` standalone — Task 2.
- ⚠️ `s3:bucket:create / :drop` — **deferred to follow-up PR** (no existing implementation; AWS SDK / sigv4 / mc-shellout is a separate scope decision).

**Placeholders:** None. Every step has concrete code or a precise "look at existing pattern in <file>" pointer.

**Type consistency:**
- `HasSetup()` method signature consistent with `HasServices()` / `HasAnyEnv()`.
- `ApplySetupStep` has a unique type name — no collision.
- `buildSetupEnv(phpVersion string) []string` is the only new helper signature in this PR.

**Deliberate notes for future readers:**
- **`pv s3:bucket:create / :drop` is the one spec divergence in PR 3.** It belongs in pv.yml's roadmap but its implementation has its own design questions (SDK choice, sigv4 inline, or `mc` shellout). Treating it as a small standalone PR after PR 3 keeps PR 3 reviewable.
- **PHP bin dir prepended to PATH, not replacing PATH.** Setup commands often need `composer`, `bun`, `pv`, etc. — those stay reachable via the user's host PATH. Only `php` is pinned.
- **`bash -c` per line means variables don't persist.** `setup: ["export FOO=1", "echo $FOO"]` would print empty. If users want shared state, they join lines with `&&` or `;` inside one entry. Documented in the step's godoc.
- **`ApplySetupStep` runs LAST.** This is the right place: by the time it runs, pv has bound services, written templated env, minted certs, generated Caddy config. The user's `composer install` + `php artisan migrate` can rely on all of that being in place.
- **The 6-step gate is independent of PR 2's gates.** A project can declare `services:` (PR 2 services gate fires), `env:` (PR 2 env gate fires), AND `setup:` (PR 3 gates fire) — all three behaviors compose without conflict.

## Open questions

1. **`phpenv.PHPPath(version)` signature.** The plan assumes it returns `string`. If it's a method on a type, or returns `(string, error)`, adjust `buildSetupEnv` accordingly. Settled during Task 3 implementation by reading `internal/phpenv/`.
2. **MySQL `InstalledVersions` ordering.** The `pv mysql:db:create` command picks the "highest installed" — assumes the slice is sorted ascending. If it's not sorted, sort before picking. Settled during Task 2.
3. **`DropDatabase` already exists for either engine?** If yes, skip the add. Settled by reading the existing `database.go` for each engine at Task 2's start.
