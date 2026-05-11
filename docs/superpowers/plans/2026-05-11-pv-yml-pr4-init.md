# pv.yml PR 4 — `pv init` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land `pv init` — a new top-level command that detects the project type and writes a sensible default `pv.yml`. This is the migration tool that makes PR 5 (the breaking change deleting the legacy pipeline) safe: users on existing pv setups can run `pv init` in each linked project, review the generated config, commit it, and continue working.

**Architecture:** Reuses the existing `detection.Detect(path)` for project-type discovery. A new `internal/initgen/` package owns per-type pv.yml generators that return a YAML string. Each generator hand-builds the YAML via `strings.Builder` so we get readable file output with comments and intentional ordering — `yaml.Marshal(*ProjectConfig)` would lose both. `cmd/init.go` is the cobra command: takes optional `[path]` (default cwd), `--force` flag, picks highest-installed postgres/mysql versions (or omits the block if none installed), and writes the file with a `ui.Success` notice.

**Tech Stack:** Go 1.x, cobra, existing `internal/detection`, `internal/postgres`, `internal/mysql`, `internal/projectenv`.

**Spec reference:** `docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md` (PR 4 section).

**Scope deviation from spec:** The spec mentions Statamic, Symfony, and Node detection. `detection.Detect()` only handles `laravel-octane`, `laravel`, `php`, `static`, `""`. Statamic projects detect as `laravel` (true — Statamic ships as a Laravel package). Symfony / Node templates are deferred to a follow-up PR that would also need to extend `detection.Detect()` itself. PR 4 ships generators for the four real types.

**Base commit:** `2c7cdee` on `main`. Branch: `feat/pvyml-init`.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `internal/initgen/initgen.go` | Create | `Generate(opts Options) string` — picks per-type template, builds YAML |
| `internal/initgen/laravel.go` | Create | Laravel + Laravel-Octane template (they're the same — Octane install is user-controlled, not auto) |
| `internal/initgen/php.go` | Create | Generic PHP template |
| `internal/initgen/static.go` | Create | Static-site template |
| `internal/initgen/unknown.go` | Create | Bare `php:` only template for unknown project types |
| `internal/initgen/initgen_test.go` | Create | Per-template unit tests asserting YAML parses + has expected fields |
| `cmd/init.go` | Create | `pv init` cobra command — wires detection, generator, file write |
| `cmd/init_test.go` | Create | End-to-end tests using `t.Setenv("HOME", ...)` pattern |
| `cmd/root.go` | Modify | One line: `rootCmd.AddCommand(initCmd)` in init() |

---

## Generated pv.yml shape (Laravel example)

For a project named `myapp` on a system with postgres 18 installed:

```yaml
php: "8.4"

aliases:
  # Add extra hostnames here; pv mints a TLS cert per alias.
  # - admin.myapp.test

env:
  APP_URL: "{{ .site_url }}"

postgresql:
  version: "18"
  env:
    DB_CONNECTION: pgsql
    DB_HOST: "{{ .host }}"
    DB_PORT: "{{ .port }}"
    DB_DATABASE: myapp
    DB_USERNAME: "{{ .username }}"
    DB_PASSWORD: "{{ .password }}"

setup:
  - cp .env.example .env
  - pv postgres:db:create myapp
  - composer install
  - php artisan key:generate
  - php artisan migrate
```

(For mysql, swap the `postgresql:` block for `mysql:` with `DB_CONNECTION: mysql`, `DB_USERNAME: root`, `DB_PASSWORD: ""`.)

---

## Database-block decision logic

`pv init` picks the database block based on what's installed:

| postgres installed? | mysql installed? | `--mysql` flag? | Result |
|---|---|---|---|
| Yes | No | n/a | postgres block (highest major) |
| No | Yes | n/a | mysql block (highest version) |
| Yes | Yes | false | postgres block |
| Yes | Yes | true | mysql block |
| No | No | n/a | No DB block (and no `pv ...:db:create` in setup, no migrate) |

When no DB engine is installed, the `setup:` block contains just `cp .env.example .env`, `composer install`, `php artisan key:generate`. The user can add `php artisan migrate` and a DB block by hand once they've installed an engine.

---

## Task 1: `initgen` package — per-type generators + tests

**Files:**
- Create: `internal/initgen/initgen.go`
- Create: `internal/initgen/laravel.go`
- Create: `internal/initgen/php.go`
- Create: `internal/initgen/static.go`
- Create: `internal/initgen/unknown.go`
- Create: `internal/initgen/initgen_test.go`

The package exports one entry point (`Generate(opts) string`) and one Options struct. Internal helpers per type. Each generator hand-builds YAML via `strings.Builder` so the output has comments and clean formatting.

- [ ] **Step 1.1: Read existing detection types**

```bash
cat internal/detection/detect.go
```

Confirm the return values are exactly `"laravel-octane"`, `"laravel"`, `"php"`, `"static"`, `""`. Adapt the plan if reality differs.

- [ ] **Step 1.2: Create the Options struct and entry point**

Create `internal/initgen/initgen.go`:

```go
// Package initgen builds default pv.yml content for newly-detected
// projects. Per-type templates hand-build YAML so the generated file
// carries comments and stable ordering — yaml.Marshal would lose both.
package initgen

// Options captures everything Generate needs to produce a per-type
// pv.yml. ProjectName must already be sanitized (call
// projectenv.SanitizeProjectName upstream).
type Options struct {
	// ProjectType matches detection.Detect(): "laravel-octane",
	// "laravel", "php", "static", or "" (unknown).
	ProjectType string

	// ProjectName is the sanitized project name. Used as the literal
	// DB_DATABASE value and as the argument to `pv postgres:db:create`
	// in the setup: block.
	ProjectName string

	// PHP is the version string for the top-level `php:` field.
	PHP string

	// Postgres, if non-empty, is the major version (e.g., "18") to
	// generate a postgresql: block for. Empty means "skip postgres
	// block."
	Postgres string

	// Mysql, if non-empty, is the version string (e.g., "8.4") to
	// generate a mysql: block for. Empty means "skip mysql block."
	// Postgres takes precedence if both are set; caller is responsible
	// for picking one.
	Mysql string
}

// Generate returns the YAML string for opts. Always emits valid YAML
// the existing LoadProjectConfig can parse round-trip.
func Generate(opts Options) string {
	switch opts.ProjectType {
	case "laravel", "laravel-octane":
		return laravel(opts)
	case "php":
		return php(opts)
	case "static":
		return static(opts)
	default:
		return unknown(opts)
	}
}
```

- [ ] **Step 1.3: Failing test — Generate produces valid YAML for Laravel**

Create `internal/initgen/initgen_test.go`:

```go
package initgen

import (
	"strings"
	"testing"

	"gopkg.in/yaml.v3"

	"github.com/prvious/pv/internal/config"
)

// parseGenerated rounds the generated YAML through the actual pv.yml
// parser. This catches malformed output, wrong field names, and yaml
// syntax errors in one assertion.
func parseGenerated(t *testing.T, body string) *config.ProjectConfig {
	t.Helper()
	var cfg config.ProjectConfig
	if err := yaml.Unmarshal([]byte(body), &cfg); err != nil {
		t.Fatalf("Unmarshal generated pv.yml: %v\n--- body ---\n%s", err, body)
	}
	return &cfg
}

func TestGenerate_LaravelWithPostgres(t *testing.T) {
	body := Generate(Options{
		ProjectType: "laravel",
		ProjectName: "myapp",
		PHP:         "8.4",
		Postgres:    "18",
	})
	cfg := parseGenerated(t, body)

	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	if cfg.Postgresql == nil {
		t.Fatal("Postgresql is nil, want declared")
	}
	if cfg.Postgresql.Version != "18" {
		t.Errorf("Postgresql.Version = %q, want %q", cfg.Postgresql.Version, "18")
	}
	if got := cfg.Postgresql.Env["DB_DATABASE"]; got != "myapp" {
		t.Errorf("DB_DATABASE = %q, want %q", got, "myapp")
	}
	if got := cfg.Postgresql.Env["DB_HOST"]; got != "{{ .host }}" {
		t.Errorf("DB_HOST = %q, want template", got)
	}
	if got := cfg.Env["APP_URL"]; got != "{{ .site_url }}" {
		t.Errorf("APP_URL = %q, want template", got)
	}

	// Setup includes db create + migrate
	joined := strings.Join(cfg.Setup, "\n")
	for _, want := range []string{"cp .env.example .env", "pv postgres:db:create myapp", "composer install", "php artisan key:generate", "php artisan migrate"} {
		if !strings.Contains(joined, want) {
			t.Errorf("setup missing %q\nsetup: %v", want, cfg.Setup)
		}
	}
}
```

- [ ] **Step 1.4: Run, verify FAIL**

```
go test ./internal/initgen/ -run TestGenerate_LaravelWithPostgres -v
```
Expected: FAIL — `undefined: Generate` (or `undefined: laravel` once Generate exists).

- [ ] **Step 1.5: Implement the Laravel generator**

Create `internal/initgen/laravel.go`:

```go
package initgen

import (
	"fmt"
	"strings"
)

func laravel(opts Options) string {
	var b strings.Builder

	fmt.Fprintf(&b, "php: %q\n\n", opts.PHP)

	b.WriteString("# Additional hostnames Caddy will serve for this project, each\n")
	b.WriteString("# with its own TLS cert. Hostnames outside *.{project}.test\n")
	b.WriteString("# (the wildcard SAN) make the most sense here.\n")
	b.WriteString("aliases:\n")
	b.WriteString("  # - admin.")
	b.WriteString(opts.ProjectName)
	b.WriteString(".test\n\n")

	b.WriteString("env:\n")
	b.WriteString("  APP_URL: \"{{ .site_url }}\"\n\n")

	switch {
	case opts.Postgres != "":
		writePostgresBlock(&b, opts)
		b.WriteString("\n")
	case opts.Mysql != "":
		writeMysqlBlock(&b, opts)
		b.WriteString("\n")
	}

	b.WriteString("# Each line runs in its own bash -c with the pinned PHP on PATH.\n")
	b.WriteString("# Fail-fast on first non-zero exit.\n")
	b.WriteString("setup:\n")
	b.WriteString("  - cp .env.example .env\n")
	if opts.Postgres != "" {
		fmt.Fprintf(&b, "  - pv postgres:db:create %s\n", opts.ProjectName)
	}
	if opts.Mysql != "" {
		fmt.Fprintf(&b, "  - pv mysql:db:create %s\n", opts.ProjectName)
	}
	b.WriteString("  - composer install\n")
	b.WriteString("  - php artisan key:generate\n")
	if opts.Postgres != "" || opts.Mysql != "" {
		b.WriteString("  - php artisan migrate\n")
	}

	return b.String()
}

// writePostgresBlock emits the postgresql: block with Laravel-shaped
// env keys. Caller is responsible for the trailing blank line.
func writePostgresBlock(b *strings.Builder, opts Options) {
	fmt.Fprintf(b, "postgresql:\n  version: %q\n  env:\n", opts.Postgres)
	b.WriteString("    DB_CONNECTION: pgsql\n")
	b.WriteString("    DB_HOST: \"{{ .host }}\"\n")
	b.WriteString("    DB_PORT: \"{{ .port }}\"\n")
	fmt.Fprintf(b, "    DB_DATABASE: %s\n", opts.ProjectName)
	b.WriteString("    DB_USERNAME: \"{{ .username }}\"\n")
	b.WriteString("    DB_PASSWORD: \"{{ .password }}\"\n")
}

// writeMysqlBlock emits the mysql: block with Laravel-shaped env keys.
func writeMysqlBlock(b *strings.Builder, opts Options) {
	fmt.Fprintf(b, "mysql:\n  version: %q\n  env:\n", opts.Mysql)
	b.WriteString("    DB_CONNECTION: mysql\n")
	b.WriteString("    DB_HOST: \"{{ .host }}\"\n")
	b.WriteString("    DB_PORT: \"{{ .port }}\"\n")
	fmt.Fprintf(b, "    DB_DATABASE: %s\n", opts.ProjectName)
	b.WriteString("    DB_USERNAME: \"{{ .username }}\"\n")
	b.WriteString("    DB_PASSWORD: \"{{ .password }}\"\n")
}
```

- [ ] **Step 1.6: Verify the test passes**

```
go test ./internal/initgen/ -run TestGenerate_LaravelWithPostgres -v
```
Expected: PASS.

- [ ] **Step 1.7: Add tests for Laravel with mysql, Laravel without DB, and the rest**

Append to `internal/initgen/initgen_test.go`:

```go
func TestGenerate_LaravelWithMysql(t *testing.T) {
	body := Generate(Options{
		ProjectType: "laravel",
		ProjectName: "myapp",
		PHP:         "8.4",
		Mysql:       "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.Mysql == nil || cfg.Mysql.Version != "8.4" {
		t.Fatalf("Mysql = %+v, want version 8.4", cfg.Mysql)
	}
	if cfg.Postgresql != nil {
		t.Errorf("Postgresql should be nil when only Mysql is requested")
	}
	if got := cfg.Mysql.Env["DB_CONNECTION"]; got != "mysql" {
		t.Errorf("DB_CONNECTION = %q, want mysql", got)
	}
	joined := strings.Join(cfg.Setup, "\n")
	if !strings.Contains(joined, "pv mysql:db:create myapp") {
		t.Errorf("setup missing mysql db create:\n%v", cfg.Setup)
	}
}

func TestGenerate_LaravelWithoutDB(t *testing.T) {
	body := Generate(Options{
		ProjectType: "laravel",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.Postgresql != nil || cfg.Mysql != nil {
		t.Errorf("No DB block expected; got Postgresql=%v Mysql=%v", cfg.Postgresql, cfg.Mysql)
	}
	joined := strings.Join(cfg.Setup, "\n")
	if strings.Contains(joined, "migrate") {
		t.Errorf("setup should not include migrate when no DB:\n%v", cfg.Setup)
	}
	if strings.Contains(joined, "db:create") {
		t.Errorf("setup should not include db:create when no DB:\n%v", cfg.Setup)
	}
}

func TestGenerate_PHP(t *testing.T) {
	body := Generate(Options{
		ProjectType: "php",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	joined := strings.Join(cfg.Setup, "\n")
	if !strings.Contains(joined, "composer install") {
		t.Errorf("setup missing composer install:\n%v", cfg.Setup)
	}
	if strings.Contains(joined, "artisan") {
		t.Errorf("generic PHP setup should not reference artisan:\n%v", cfg.Setup)
	}
}

func TestGenerate_Static(t *testing.T) {
	body := Generate(Options{
		ProjectType: "static",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	if len(cfg.Setup) != 0 {
		t.Errorf("static project should have empty setup, got: %v", cfg.Setup)
	}
}

func TestGenerate_Unknown(t *testing.T) {
	body := Generate(Options{
		ProjectType: "",
		ProjectName: "myapp",
		PHP:         "8.4",
	})
	cfg := parseGenerated(t, body)
	if cfg.PHP != "8.4" {
		t.Errorf("PHP = %q, want %q", cfg.PHP, "8.4")
	}
	if len(cfg.Setup) != 0 {
		t.Errorf("unknown project should have empty setup, got: %v", cfg.Setup)
	}
}

func TestGenerate_OctaneSameAsLaravel(t *testing.T) {
	octane := Generate(Options{ProjectType: "laravel-octane", ProjectName: "myapp", PHP: "8.4", Postgres: "18"})
	laravel := Generate(Options{ProjectType: "laravel", ProjectName: "myapp", PHP: "8.4", Postgres: "18"})
	if octane != laravel {
		t.Errorf("laravel-octane should produce identical output to laravel (octane install is user-controlled, not auto-templated)")
	}
}
```

- [ ] **Step 1.8: Create the other three generators**

Create `internal/initgen/php.go`:

```go
package initgen

import (
	"fmt"
	"strings"
)

func php(opts Options) string {
	var b strings.Builder
	fmt.Fprintf(&b, "php: %q\n\n", opts.PHP)
	b.WriteString("# Each line runs in its own bash -c with the pinned PHP on PATH.\n")
	b.WriteString("setup:\n")
	b.WriteString("  - composer install\n")
	return b.String()
}
```

Create `internal/initgen/static.go`:

```go
package initgen

import (
	"fmt"
	"strings"
)

func static(opts Options) string {
	var b strings.Builder
	fmt.Fprintf(&b, "php: %q\n", opts.PHP)
	b.WriteString("\n")
	b.WriteString("# Static site — no setup pipeline needed.\n")
	b.WriteString("# Add `aliases:`, `env:`, or `setup:` blocks as your project grows.\n")
	return b.String()
}
```

Create `internal/initgen/unknown.go`:

```go
package initgen

import (
	"fmt"
	"strings"
)

func unknown(opts Options) string {
	var b strings.Builder
	fmt.Fprintf(&b, "php: %q\n", opts.PHP)
	b.WriteString("\n")
	b.WriteString("# pv couldn't identify this project's type. Add the blocks you need:\n")
	b.WriteString("# - aliases:    extra hostnames Caddy should serve\n")
	b.WriteString("# - env:        project-level env keys (e.g., APP_URL)\n")
	b.WriteString("# - postgresql / mysql / redis / mailpit / rustfs: backing service declarations\n")
	b.WriteString("# - setup:      shell commands to run after `pv link`\n")
	return b.String()
}
```

- [ ] **Step 1.9: Verify all generator tests pass**

```
go test ./internal/initgen/ -v
```
Expected: 6 tests PASS (`LaravelWithPostgres`, `LaravelWithMysql`, `LaravelWithoutDB`, `PHP`, `Static`, `Unknown`, `OctaneSameAsLaravel`).

- [ ] **Step 1.10: gofmt + vet + build**

```
gofmt -w internal/initgen/
go vet ./internal/initgen/...
go build ./...
```
Expected: clean.

- [ ] **Step 1.11: Commit**

```bash
git add internal/initgen/
git commit -m "$(cat <<'EOF'
feat(initgen): per-type pv.yml templates for laravel/php/static/unknown

Hand-built YAML strings per project type so the generated file
carries comments and stable ordering — yaml.Marshal would lose
both. Laravel template emits postgresql or mysql block when the
matching engine is available; laravel-octane uses the same template
as laravel because Octane install is user-controlled (not
auto-applied). Round-trip tested through LoadProjectConfig so a
syntactically broken template fails the unit test instead of
shipping to users.
EOF
)"
```

---

## Task 2: `pv init` cobra command

**Files:**
- Create: `cmd/init.go`
- Modify: `cmd/root.go` (one line)

The command:
- Default path: cwd.
- Optional positional `[path]` arg (like `pv link`).
- `--force / -f` flag: overwrite an existing pv.yml.
- `--mysql` flag: prefer mysql over postgres when both are installed.
- Detects type via `detection.Detect(path)`.
- Picks highest installed postgres major / mysql version.
- Writes `<path>/pv.yml`.
- Prints `ui.Success(...)` notice + the path.

- [ ] **Step 2.1: Read existing top-level command for style**

```bash
cat cmd/start.go
cat cmd/link.go | head -120
```

Capture: import grouping, var name conventions (`startCmd`, `linkCmd`), flag declaration style, GroupID values.

- [ ] **Step 2.2: Create `cmd/init.go`**

```go
package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/prvious/pv/internal/config"
	"github.com/prvious/pv/internal/detection"
	"github.com/prvious/pv/internal/initgen"
	"github.com/prvious/pv/internal/mysql"
	"github.com/prvious/pv/internal/postgres"
	"github.com/prvious/pv/internal/projectenv"
	"github.com/prvious/pv/internal/ui"
	"github.com/spf13/cobra"
)

var (
	initForce bool
	initMysql bool
)

var initCmd = &cobra.Command{
	Use:     "init [path]",
	GroupID: "core",
	Short:   "Generate a default pv.yml for the project",
	Long: `Detect the project type and write a pv.yml with sensible defaults.
Refuses to overwrite an existing pv.yml unless --force is set.

Designed to be reviewed and committed: the file is the contract
between your project and pv.`,
	Example: `  pv init
  pv init /path/to/project
  pv init --mysql           # prefer mysql when both postgres + mysql are installed
  pv init --force           # overwrite an existing pv.yml`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(cmd *cobra.Command, args []string) error {
		projectPath, err := resolveInitPath(args)
		if err != nil {
			return err
		}

		ymlPath := filepath.Join(projectPath, config.ProjectConfigFilename)
		if _, statErr := os.Stat(ymlPath); statErr == nil && !initForce {
			return fmt.Errorf("pv.yml already exists at %s — pass --force to overwrite", ymlPath)
		}

		projectType := detection.Detect(projectPath)
		projectName := projectenv.SanitizeProjectName(filepath.Base(projectPath))

		opts := initgen.Options{
			ProjectType: projectType,
			ProjectName: projectName,
			PHP:         resolveInitPHP(projectPath),
			Postgres:    resolveInitPostgres(initMysql),
			Mysql:       resolveInitMysql(initMysql),
		}

		body := initgen.Generate(opts)
		if err := os.WriteFile(ymlPath, []byte(body), 0o644); err != nil {
			return fmt.Errorf("write pv.yml: %w", err)
		}

		ui.Success(fmt.Sprintf("Generated %s", ymlPath))
		ui.Subtle(fmt.Sprintf("Detected project type: %s", labelForType(projectType)))
		ui.Subtle("Review the file and adjust before running `pv link`.")
		return nil
	},
}

// resolveInitPath returns the absolute path of the project we're
// generating pv.yml for: the args[0] if given, else cwd. Validates
// that the path is a directory.
func resolveInitPath(args []string) (string, error) {
	raw := "."
	if len(args) > 0 {
		raw = args[0]
	}
	abs, err := filepath.Abs(raw)
	if err != nil {
		return "", fmt.Errorf("resolve path %q: %w", raw, err)
	}
	info, err := os.Stat(abs)
	if err != nil {
		return "", fmt.Errorf("stat %s: %w", abs, err)
	}
	if !info.IsDir() {
		return "", fmt.Errorf("%s is not a directory", abs)
	}
	return abs, nil
}

// resolveInitPHP returns the PHP version to pin in the generated
// pv.yml. Prefers composer.json's require.php if parseable, otherwise
// the user's global default ("8.4" today, but read from settings).
func resolveInitPHP(projectPath string) string {
	// Best-effort: parse composer.json's require.php constraint.
	if v := phpFromComposer(projectPath); v != "" {
		return v
	}
	settings, err := config.LoadSettings()
	if err != nil || settings == nil {
		return "8.4"
	}
	if settings.Defaults.PHP != "" {
		return settings.Defaults.PHP
	}
	return "8.4"
}

// phpFromComposer reads composer.json's require.php and returns a
// concrete major.minor (e.g., "8.4") when the constraint allows it.
// Returns "" on any parse failure or constraint that doesn't pin a
// version cleanly. Implementation is intentionally conservative —
// fall back to the global default rather than guess wrong.
func phpFromComposer(projectPath string) string {
	// Caller may find that internal/detection has a helper for this
	// already (it reads composer.json to find laravel/framework /
	// laravel/octane). If detection exports the parsed composer.json,
	// use that. Otherwise: read composer.json, json.Unmarshal into
	// {Require map[string]string}, look at Require["php"], and try
	// to extract a major.minor from constraints like "^8.2", "~8.2.0",
	// ">=8.2 <8.5". For PR 4 simplicity: only match `^X.Y` and `~X.Y`,
	// return X.Y. Anything else → "".
	//
	// TODO during implementation: read the actual detection.go and
	// reuse its composer parser if there's one.
	return ""
}

// resolveInitPostgres returns the highest installed postgres major,
// or "" if mysql is preferred OR if no postgres is installed.
func resolveInitPostgres(preferMysql bool) string {
	majors, _ := postgres.InstalledMajors()
	if len(majors) == 0 {
		return ""
	}
	if preferMysql {
		// Check if mysql is also installed; if so, defer to mysql by
		// returning "" here.
		versions, _ := mysql.InstalledVersions()
		if len(versions) > 0 {
			return ""
		}
	}
	return majors[len(majors)-1]
}

// resolveInitMysql returns the highest installed mysql version, or
// "" when postgres should win (i.e., postgres is installed and
// preferMysql is false).
func resolveInitMysql(preferMysql bool) string {
	versions, _ := mysql.InstalledVersions()
	if len(versions) == 0 {
		return ""
	}
	if !preferMysql {
		majors, _ := postgres.InstalledMajors()
		if len(majors) > 0 {
			return ""
		}
	}
	return versions[len(versions)-1]
}

func labelForType(t string) string {
	switch t {
	case "laravel-octane":
		return "Laravel + Octane"
	case "laravel":
		return "Laravel"
	case "php":
		return "Generic PHP / Composer"
	case "static":
		return "Static site"
	default:
		return "Unknown"
	}
}

func init() {
	initCmd.Flags().BoolVarP(&initForce, "force", "f", false, "Overwrite an existing pv.yml")
	initCmd.Flags().BoolVar(&initMysql, "mysql", false, "Prefer MySQL when both postgres and mysql are installed")
}
```

- [ ] **Step 2.3: Wire into `cmd/root.go`**

In `cmd/root.go`, find the `init()` function (or wherever existing commands are added with `rootCmd.AddCommand(...)`). Add:

```go
rootCmd.AddCommand(initCmd)
```

Place it next to other core commands like `linkCmd` / `unlinkCmd` (alphabetical or grouped — match existing style).

- [ ] **Step 2.4: Verify project-wide**

```
gofmt -w cmd/
go vet ./...
go build ./...
go test ./...
```
Expected: clean.

- [ ] **Step 2.5: Smoke test the new command**

```bash
go build -o /tmp/pv-init-smoke .
/tmp/pv-init-smoke init --help
/tmp/pv-init-smoke --help | grep init
rm /tmp/pv-init-smoke
```
Expected: help screen renders with `[path]` arg, `--force` flag, `--mysql` flag, and Examples section. Root help lists `init` under "Core Commands" (or similar group).

- [ ] **Step 2.6: Commit**

```bash
git add cmd/init.go cmd/root.go
git commit -m "$(cat <<'EOF'
feat(init): pv init command — generate default pv.yml per project type

Detects the project type via internal/detection, picks the highest
installed postgres major (or mysql with --mysql when both are
present), writes pv.yml in the project root via initgen.Generate.
Refuses to overwrite an existing pv.yml unless --force is set.
PHP version comes from composer.json's require.php when parseable,
otherwise the global default in settings.yml.
EOF
)"
```

---

## Task 3: End-to-end cobra tests

**Files:**
- Create: `cmd/init_test.go`

Mirror the `cmd/link_test.go` pattern: `t.TempDir()` for the project dir, `t.Setenv("HOME", t.TempDir())` for ~/.pv isolation, construct a fresh `*cobra.Command` so tests don't share state with each other or with the global `initCmd`.

- [ ] **Step 3.1: Read `cmd/link_test.go` patterns**

```bash
head -100 cmd/link_test.go
```

Note the `newLinkCmd()` helper and `t.Setenv("HOME", ...)` pattern. Adapt for `init`.

- [ ] **Step 3.2: Add failing tests**

Create `cmd/init_test.go`:

```go
package cmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/prvious/pv/internal/config"
	"github.com/spf13/cobra"
)

// newInitCmd builds a fresh cobra root + init subcommand for tests so
// state from the package-level initCmd / flags doesn't leak between
// tests.
func newInitCmd() *cobra.Command {
	root := &cobra.Command{Use: "pv", SilenceErrors: true, SilenceUsage: true}
	root.AddCommand(initCmd)
	return root
}

func TestInit_LaravelProject(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := t.TempDir()
	// Laravel marker: composer.json with laravel/framework
	composer := `{"require":{"laravel/framework":"^11.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0o644); err != nil {
		t.Fatal(err)
	}

	// Reset flags for test isolation
	initForce = false
	initMysql = false

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	if err != nil {
		t.Fatalf("read pv.yml: %v", err)
	}
	s := string(body)
	for _, want := range []string{"php: ", "env:", "APP_URL", "setup:", "composer install", "php artisan key:generate"} {
		if !strings.Contains(s, want) {
			t.Errorf("pv.yml missing %q\n--- contents ---\n%s", want, s)
		}
	}
}

func TestInit_RefusesWhenPvYmlExists(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(projDir, config.ProjectConfigFilename), []byte("php: \"8.4\"\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	initForce = false
	initMysql = false

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	err := cmd.Execute()
	if err == nil {
		t.Fatal("Execute: want error when pv.yml exists, got nil")
	}
	if !strings.Contains(err.Error(), "--force") {
		t.Errorf("err = %v, want it to suggest --force", err)
	}
}

func TestInit_ForceOverwrites(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := t.TempDir()
	existing := "php: \"7.4\"\n# this should be replaced\n"
	if err := os.WriteFile(filepath.Join(projDir, config.ProjectConfigFilename), []byte(existing), 0o644); err != nil {
		t.Fatal(err)
	}

	initForce = true
	initMysql = false

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir, "--force"})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init --force: %v", err)
	}

	body, err := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	if err != nil {
		t.Fatal(err)
	}
	s := string(body)
	if strings.Contains(s, "this should be replaced") {
		t.Errorf("pv.yml still contains the old content:\n%s", s)
	}
	if !strings.Contains(s, "php: ") {
		t.Errorf("pv.yml looks malformed:\n%s", s)
	}
}

func TestInit_GenericPHP(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := t.TempDir()
	// composer.json without laravel/framework → "php" type
	composer := `{"require":{"monolog/monolog":"^3.0"}}`
	if err := os.WriteFile(filepath.Join(projDir, "composer.json"), []byte(composer), 0o644); err != nil {
		t.Fatal(err)
	}

	initForce = false
	initMysql = false

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init: %v", err)
	}

	body, _ := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	s := string(body)
	if !strings.Contains(s, "composer install") {
		t.Errorf("pv.yml should include composer install:\n%s", s)
	}
	if strings.Contains(s, "artisan") {
		t.Errorf("generic PHP pv.yml should NOT reference artisan:\n%s", s)
	}
}

func TestInit_UnknownProject(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	writeDefaultSettings(t)

	projDir := t.TempDir()
	// No markers; detection.Detect returns "" → "unknown" path

	initForce = false
	initMysql = false

	cmd := newInitCmd()
	cmd.SetArgs([]string{"init", projDir})
	if err := cmd.Execute(); err != nil {
		t.Fatalf("init: %v", err)
	}

	body, _ := os.ReadFile(filepath.Join(projDir, config.ProjectConfigFilename))
	s := string(body)
	if !strings.Contains(s, "php: ") {
		t.Errorf("pv.yml should have at least the php: field:\n%s", s)
	}
}
```

`writeDefaultSettings(t)` already exists in `cmd/link_test.go` per the explorer; we re-use it. If it isn't accessible from this file (different package, somehow), copy/adapt the helper.

- [ ] **Step 3.3: Run, verify FAIL (or partial pass)**

```
go test ./cmd/ -run TestInit -v
```
Expected: each test PASSES if Task 2's implementation is correct; FAILs if there's a wiring or flag bug. Investigate any failure before proceeding.

- [ ] **Step 3.4: Verify project-wide**

```
go test ./... -count=1
gofmt -l .
go vet ./...
go build ./...
```
Expected: clean.

- [ ] **Step 3.5: Commit**

```bash
git add cmd/init_test.go
git commit -m "$(cat <<'EOF'
test(init): end-to-end cobra tests for pv init

Covers: Laravel project generates expected setup steps + env keys,
generic PHP path skips artisan references, --force overwrites
existing pv.yml, default behavior refuses overwrite, unknown
project type writes a minimal valid pv.yml. All tests use
t.Setenv("HOME", t.TempDir()) for filesystem isolation.
EOF
)"
```

---

## Task 4: Final verification sweep

**Files:** none modified.

- [ ] **Step 4.1: Project-wide gates**

```
gofmt -l .
go vet ./...
go test ./... -count=1
go build ./...
```
Expected: every gate clean.

- [ ] **Step 4.2: Smoke-test the binary against a real Laravel layout**

```bash
go build -o /tmp/pv-pr4-smoke .

# 1. Empty dir → unknown type, minimal pv.yml
tmp=$(mktemp -d)
/tmp/pv-pr4-smoke init "$tmp"
cat "$tmp/pv.yml"
rm -rf "$tmp"

# 2. Laravel layout → full pv.yml
tmp=$(mktemp -d)
echo '{"require":{"laravel/framework":"^11.0"}}' > "$tmp/composer.json"
/tmp/pv-pr4-smoke init "$tmp"
cat "$tmp/pv.yml"

# 3. --force overwrite
/tmp/pv-pr4-smoke init "$tmp"  # should fail
/tmp/pv-pr4-smoke init --force "$tmp"  # should succeed
rm -rf "$tmp"

rm /tmp/pv-pr4-smoke
```

Confirm: minimal pv.yml for empty dir, Laravel pv.yml with setup block for the Laravel project, refuse-then-force behavior works.

- [ ] **Step 4.3: Confirm commit count and clean working tree**

```
git status
git log --oneline main..HEAD
```
Expected: working tree clean; 4 commits on the branch (plan + 3 implementation).

---

## Self-Review (already applied)

**Spec coverage:**
- ✅ `pv init` as a new top-level command — Task 2.
- ✅ Project type detection — uses existing `detection.Detect()`.
- ✅ Per-type pv.yml generation — Task 1 covers laravel, laravel-octane (same as laravel), php, static, unknown.
- ⚠️ Statamic / Symfony / Node detection — deferred (`detection.Detect()` doesn't support them; out of PR 4 scope).
- ✅ Laravel template includes standard postgresql/mysql block with Laravel-shaped env, top-level APP_URL via template, setup: with the four standard commands.
- ✅ `--force` to overwrite — Task 2.
- ✅ Refuses without --force — Task 2.
- ✅ Unit tests per project type — Task 1 (5 type tests + Octane-equals-Laravel parity).
- ⚠️ E2E test (fresh Laravel skeleton + `pv init` + `pv link` produces a working project) — NOT included; the cmd-level test in Task 3 covers init in isolation. Full e2e against real pv link state would require services running, which isn't tractable in unit tests. Could land as a `scripts/e2e/init.sh` phase in a follow-up.

**Placeholders:** Two `TODO during implementation` markers in `cmd/init.go`'s `phpFromComposer` — these are explicitly flagged because the implementer needs to decide whether to read `internal/detection`'s composer parser or write a separate one. Acceptable for a plan because the function's contract (`return X.Y from require.php or "" on any failure`) is clear.

**Type consistency:**
- `initgen.Options` struct used identically in tests and the command.
- `Generate(opts Options) string` is the only exported entry point from the package.
- Project type strings (`"laravel"`, `"php"`, `"static"`, `"laravel-octane"`, `""`) match what `detection.Detect()` produces.

**Deliberate notes for future readers:**
- **Laravel-Octane shares the laravel template** because Octane install is user-controlled. Users who want Octane add `composer require laravel/octane && php artisan octane:install --server=frankenphp` to their `setup:` themselves. We don't auto-template this because (a) the Octane install asks interactive questions, (b) project teams typically commit `public/frankenphp-worker.php` once and never re-run install.
- **Generic PHP gets no env: block** because we have no idea what env keys the project uses. Setup just runs `composer install` to populate `vendor/`.
- **Static sites get a placeholder pv.yml** so users can `pv link` them and later add aliases/env as the project grows.
- **DB block omitted when neither engine is installed** rather than written with a placeholder version. Reason: pv.yml needs to be valid post-`pv link`, and a `postgresql: { version: 18 }` declaration for an uninstalled version errors. Better to omit and let the user add it after they `pv postgres:install`.

## Open questions

1. **`phpFromComposer` implementation.** Two options: reuse `internal/detection`'s composer.json parser (if it exposes one) or write a tight regex-based extractor. Decide during Task 2 implementation by reading `internal/detection/detect.go`.
2. **Should `pv init` prompt before overwrite (Y/n) instead of erroring?** Cleaner UX, more interactive code. PR 4 default is the error path because it's safer; if users hit it often, we add the prompt in a follow-up.
3. **E2E coverage** (`scripts/e2e/init.sh`): deferred. The cmd-level test covers init in isolation; a full link-and-verify is a future enhancement.
