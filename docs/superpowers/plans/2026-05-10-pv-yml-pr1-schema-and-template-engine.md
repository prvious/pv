# pv.yml PR 1 — Schema + template engine (parse-only) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the full pv.yml schema (aliases, env, service blocks, setup) plus a Go-template renderer and per-service template-var producers. No runtime behavior change yet — `pv link` and `pv setup` do not read the new fields. This unlocks PR 2 to consume the parsed data.

**Architecture:** Schema lives in `internal/config/pvyml.go` (extending the existing `ProjectConfig`). Renderer and project-level vars go in `internal/projectenv/` (already home to `ReadDotEnv`/`MergeDotEnv`/`SanitizeProjectName`). Each service grows a sibling `template_vars.go` that returns its own var map. All functions are pure — version-probing stays out so we can unit-test deterministically; runtime callers in PR 2 will pass already-probed values in.

**Tech Stack:** Go 1.x, `gopkg.in/yaml.v3` (already in `pvyml.go`), `text/template` (stdlib).

**Spec reference:** `docs/superpowers/specs/2026-05-10-pv-yml-explicit-config-design.md` — see "Schema" and "Template variables" sections.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `internal/config/pvyml.go` | Modify | Extend `ProjectConfig` with `Aliases`, `Env`, per-service `*ServiceConfig` pointers, `Setup`. Add `ServiceConfig` type. |
| `internal/config/pvyml_test.go` | Modify | Add tests for parsing every new field; keep all existing PHP-only tests green. |
| `internal/projectenv/template.go` | Create | `Render(tmplStr string, vars map[string]string) (string, error)` — Go `text/template` with `missingkey=error`. |
| `internal/projectenv/template_test.go` | Create | Substitution, literal pass-through, unknown-var error, syntax error. |
| `internal/projectenv/project_vars.go` | Create | `ProjectTemplateVars(projectName, tld string) map[string]string` — produces `site_url`, `site_host`, `tls_cert_path`, `tls_key_path`. |
| `internal/projectenv/project_vars_test.go` | Create | Default + custom TLD, path-suffix assertion. |
| `internal/postgres/template_vars.go` | Create | `TemplateVars(major, fullVersion string) (map[string]string, error)` — host, port, username, password, version, dsn. |
| `internal/postgres/template_vars_test.go` | Create | Happy path + invalid-major-propagated error. |
| `internal/mysql/template_vars.go` | Create | `TemplateVars(version, fullVersion string) (map[string]string, error)` — host, port, username, password, version, dsn. |
| `internal/mysql/template_vars_test.go` | Create | Happy path + invalid-version-propagated error. |
| `internal/redis/template_vars.go` | Create | `TemplateVars() map[string]string` — host, port, password, url. |
| `internal/redis/template_vars_test.go` | Create | Happy path. |
| `internal/mailpit/template_vars.go` | Create | `TemplateVars() map[string]string` — smtp_host, smtp_port, http_host, http_port. |
| `internal/mailpit/template_vars_test.go` | Create | Happy path. |
| `internal/rustfs/template_vars.go` | Create | `TemplateVars() map[string]string` — endpoint, access_key, secret_key, region, use_path_style. |
| `internal/rustfs/template_vars_test.go` | Create | Happy path. |

Boundary rule: each per-service `template_vars.go` depends only on its own package (port constants/functions, no external services). `internal/projectenv/project_vars.go` depends on `internal/certs/` (for cert paths) and that's it. None of these are wired into `pv link` or `pv setup` in this PR.

---

## Task 1: Extend pv.yml schema with new fields

**Files:**
- Modify: `internal/config/pvyml.go:14-16`
- Modify: `internal/config/pvyml_test.go`

The new fields are all optional. Pointer types for service blocks let downstream code distinguish "service not declared" from "service declared empty."

- [ ] **Step 1.1: Write failing test for parsing aliases**

Append to `internal/config/pvyml_test.go`:

```go
func TestLoadProjectConfig_ParsesAliases(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := "php: \"8.4\"\naliases:\n  - admin.myapp.test\n  - api.myapp.test\n"
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	want := []string{"admin.myapp.test", "api.myapp.test"}
	if len(cfg.Aliases) != len(want) {
		t.Fatalf("Aliases len = %d, want %d", len(cfg.Aliases), len(want))
	}
	for i, a := range want {
		if cfg.Aliases[i] != a {
			t.Errorf("Aliases[%d] = %q, want %q", i, cfg.Aliases[i], a)
		}
	}
}
```

- [ ] **Step 1.2: Run test, verify it fails**

Run: `go test ./internal/config/ -run TestLoadProjectConfig_ParsesAliases -v`
Expected: FAIL — `cfg.Aliases undefined (type *ProjectConfig has no field or method Aliases)`.

- [ ] **Step 1.3: Add Aliases field to ProjectConfig**

Replace lines 13–16 in `internal/config/pvyml.go`:

```go
// ProjectConfig represents the contents of a pv.yml file.
type ProjectConfig struct {
	PHP     string   `yaml:"php"`
	Aliases []string `yaml:"aliases,omitempty"`
}
```

- [ ] **Step 1.4: Verify the new test passes and existing tests still pass**

Run: `go test ./internal/config/ -v`
Expected: PASS for `TestLoadProjectConfig_ParsesAliases` and all existing tests.

- [ ] **Step 1.5: Write failing test for parsing top-level env**

Append to `internal/config/pvyml_test.go`:

```go
func TestLoadProjectConfig_ParsesTopLevelEnv(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := "php: \"8.4\"\nenv:\n  APP_URL: \"{{ .site_url }}\"\n  APP_NAME: MyApp\n"
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if got := cfg.Env["APP_URL"]; got != "{{ .site_url }}" {
		t.Errorf("Env[APP_URL] = %q, want %q", got, "{{ .site_url }}")
	}
	if got := cfg.Env["APP_NAME"]; got != "MyApp" {
		t.Errorf("Env[APP_NAME] = %q, want %q", got, "MyApp")
	}
}
```

- [ ] **Step 1.6: Run test, verify it fails**

Run: `go test ./internal/config/ -run TestLoadProjectConfig_ParsesTopLevelEnv -v`
Expected: FAIL — `cfg.Env undefined`.

- [ ] **Step 1.7: Add Env field**

In `internal/config/pvyml.go`, extend `ProjectConfig`:

```go
type ProjectConfig struct {
	PHP     string            `yaml:"php"`
	Aliases []string          `yaml:"aliases,omitempty"`
	Env     map[string]string `yaml:"env,omitempty"`
}
```

- [ ] **Step 1.8: Verify**

Run: `go test ./internal/config/ -v`
Expected: PASS (all existing + 2 new).

- [ ] **Step 1.9: Write failing test for parsing a service block**

Append:

```go
func TestLoadProjectConfig_ParsesPostgresService(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
postgresql:
  version: "18"
  env:
    DB_HOST: "{{ .host }}"
    DB_PORT: "{{ .port }}"
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Postgresql == nil {
		t.Fatal("Postgresql is nil, want declared")
	}
	if cfg.Postgresql.Version != "18" {
		t.Errorf("Postgresql.Version = %q, want %q", cfg.Postgresql.Version, "18")
	}
	if got := cfg.Postgresql.Env["DB_HOST"]; got != "{{ .host }}" {
		t.Errorf("Postgresql.Env[DB_HOST] = %q, want %q", got, "{{ .host }}")
	}
}
```

- [ ] **Step 1.10: Run test, verify it fails**

Run: `go test ./internal/config/ -run TestLoadProjectConfig_ParsesPostgresService -v`
Expected: FAIL — `cfg.Postgresql undefined`.

- [ ] **Step 1.11: Add ServiceConfig type + all five service fields**

Extend `internal/config/pvyml.go`:

```go
// ProjectConfig represents the contents of a pv.yml file.
type ProjectConfig struct {
	PHP        string            `yaml:"php"`
	Aliases    []string          `yaml:"aliases,omitempty"`
	Env        map[string]string `yaml:"env,omitempty"`
	Postgresql *ServiceConfig    `yaml:"postgresql,omitempty"`
	Mysql      *ServiceConfig    `yaml:"mysql,omitempty"`
	Redis      *ServiceConfig    `yaml:"redis,omitempty"`
	Mailpit    *ServiceConfig    `yaml:"mailpit,omitempty"`
	Rustfs     *ServiceConfig    `yaml:"rustfs,omitempty"`
	Setup      []string          `yaml:"setup,omitempty"`
}

// ServiceConfig declares a backing service a project depends on.
// Version is required for postgresql and mysql (multi-version aware);
// optional and ignored for redis, mailpit, rustfs (single bundled version).
type ServiceConfig struct {
	Version string            `yaml:"version,omitempty"`
	Env     map[string]string `yaml:"env,omitempty"`
}
```

- [ ] **Step 1.12: Verify**

Run: `go test ./internal/config/ -v`
Expected: PASS.

- [ ] **Step 1.13: Write tests for the other four service blocks and setup**

Append:

```go
func TestLoadProjectConfig_ParsesMysqlService(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
mysql:
  version: "8.0"
  env:
    DB_HOST: "{{ .host }}"
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Mysql == nil || cfg.Mysql.Version != "8.0" {
		t.Fatalf("Mysql = %+v, want version 8.0", cfg.Mysql)
	}
}

func TestLoadProjectConfig_ParsesRedisMailpitRustfs(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
redis:
  env:
    REDIS_HOST: "{{ .host }}"
mailpit:
  env:
    MAIL_HOST: "{{ .smtp_host }}"
rustfs:
  env:
    AWS_ENDPOINT: "{{ .endpoint }}"
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Redis == nil || cfg.Redis.Env["REDIS_HOST"] != "{{ .host }}" {
		t.Errorf("Redis = %+v, want REDIS_HOST templated", cfg.Redis)
	}
	if cfg.Mailpit == nil || cfg.Mailpit.Env["MAIL_HOST"] != "{{ .smtp_host }}" {
		t.Errorf("Mailpit = %+v, want MAIL_HOST templated", cfg.Mailpit)
	}
	if cfg.Rustfs == nil || cfg.Rustfs.Env["AWS_ENDPOINT"] != "{{ .endpoint }}" {
		t.Errorf("Rustfs = %+v, want AWS_ENDPOINT templated", cfg.Rustfs)
	}
}

func TestLoadProjectConfig_ParsesSetup(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	body := `php: "8.4"
setup:
  - composer install
  - php artisan key:generate
  - php artisan migrate
`
	if err := os.WriteFile(path, []byte(body), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	want := []string{"composer install", "php artisan key:generate", "php artisan migrate"}
	if len(cfg.Setup) != len(want) {
		t.Fatalf("Setup len = %d, want %d", len(cfg.Setup), len(want))
	}
	for i, c := range want {
		if cfg.Setup[i] != c {
			t.Errorf("Setup[%d] = %q, want %q", i, cfg.Setup[i], c)
		}
	}
}

func TestLoadProjectConfig_OmittedServicesAreNil(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, ProjectConfigFilename)
	if err := os.WriteFile(path, []byte("php: \"8.4\"\n"), 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadProjectConfig(path)
	if err != nil {
		t.Fatalf("LoadProjectConfig() error = %v", err)
	}
	if cfg.Postgresql != nil || cfg.Mysql != nil || cfg.Redis != nil ||
		cfg.Mailpit != nil || cfg.Rustfs != nil {
		t.Errorf("services should be nil when undeclared, got %+v / %+v / %+v / %+v / %+v",
			cfg.Postgresql, cfg.Mysql, cfg.Redis, cfg.Mailpit, cfg.Rustfs)
	}
	if len(cfg.Aliases) != 0 || len(cfg.Env) != 0 || len(cfg.Setup) != 0 {
		t.Errorf("optional slices/maps should be empty, got aliases=%v env=%v setup=%v",
			cfg.Aliases, cfg.Env, cfg.Setup)
	}
}
```

- [ ] **Step 1.14: Run all schema tests**

Run: `go test ./internal/config/ -v`
Expected: PASS — every existing test green, every new test green.

- [ ] **Step 1.15: gofmt + vet + build**

Run: `gofmt -w internal/config/ && go vet ./internal/config/... && go build ./...`
Expected: no output.

- [ ] **Step 1.16: Commit**

```bash
git add internal/config/pvyml.go internal/config/pvyml_test.go
git commit -m "$(cat <<'EOF'
feat(config): extend pv.yml schema with aliases, env, services, setup

Adds Aliases []string, Env map[string]string, per-service
*ServiceConfig pointers (postgresql/mysql/redis/mailpit/rustfs), and
Setup []string. ServiceConfig carries optional Version + Env map.
All fields omit-empty; pointer types let downstream distinguish
"service not declared" from "service declared empty". Parse-only —
no command reads these yet.
EOF
)"
```

---

## Task 2: Add template renderer

**Files:**
- Create: `internal/projectenv/template.go`
- Create: `internal/projectenv/template_test.go`

Go stdlib `text/template` with `missingkey=error` — typo'd template vars become hard errors rather than silently producing `<no value>`.

- [ ] **Step 2.1: Write failing test**

Create `internal/projectenv/template_test.go`:

```go
package projectenv

import (
	"strings"
	"testing"
)

func TestRender_SubstitutesVars(t *testing.T) {
	got, err := Render("host={{ .host }} port={{ .port }}", map[string]string{
		"host": "127.0.0.1",
		"port": "5432",
	})
	if err != nil {
		t.Fatalf("Render() error = %v", err)
	}
	want := "host=127.0.0.1 port=5432"
	if got != want {
		t.Errorf("Render() = %q, want %q", got, want)
	}
}

func TestRender_PassesThroughLiteralStrings(t *testing.T) {
	got, err := Render("MyApp", map[string]string{})
	if err != nil {
		t.Fatalf("Render() error = %v", err)
	}
	if got != "MyApp" {
		t.Errorf("Render() = %q, want %q", got, "MyApp")
	}
}

func TestRender_ErrorsOnUnknownVar(t *testing.T) {
	_, err := Render("{{ .nonexistent }}", map[string]string{"other": "x"})
	if err == nil {
		t.Fatal("Render() with unknown var: want error, got nil")
	}
	if !strings.Contains(err.Error(), "nonexistent") {
		t.Errorf("error should mention the missing key, got: %v", err)
	}
}

func TestRender_ErrorsOnInvalidSyntax(t *testing.T) {
	_, err := Render("{{ .unterminated", map[string]string{})
	if err == nil {
		t.Fatal("Render() with invalid syntax: want error, got nil")
	}
}
```

- [ ] **Step 2.2: Run test, verify it fails**

Run: `go test ./internal/projectenv/ -run TestRender -v`
Expected: FAIL — `undefined: Render`.

- [ ] **Step 2.3: Implement Render**

Create `internal/projectenv/template.go`:

```go
package projectenv

import (
	"bytes"
	"text/template"
)

// Render applies a Go text/template against vars and returns the result.
// Unknown keys (typos in pv.yml) produce an error instead of silently
// rendering "<no value>" — pv.yml is a contract, surprises are bugs.
func Render(tmplStr string, vars map[string]string) (string, error) {
	t, err := template.New("pvyml").Option("missingkey=error").Parse(tmplStr)
	if err != nil {
		return "", err
	}
	var buf bytes.Buffer
	if err := t.Execute(&buf, vars); err != nil {
		return "", err
	}
	return buf.String(), nil
}
```

- [ ] **Step 2.4: Verify**

Run: `go test ./internal/projectenv/ -v`
Expected: PASS for all four Render tests, no existing test broken.

- [ ] **Step 2.5: gofmt + vet + build**

Run: `gofmt -w internal/projectenv/ && go vet ./internal/projectenv/... && go build ./...`
Expected: no output.

- [ ] **Step 2.6: Commit**

```bash
git add internal/projectenv/template.go internal/projectenv/template_test.go
git commit -m "$(cat <<'EOF'
feat(projectenv): add template renderer for pv.yml env values

Render() wraps text/template with missingkey=error so a typo in a
pv.yml env template (e.g. {{ .hosst }}) fails loud at link time
instead of writing "<no value>" into a project's .env. Not yet
wired into pv link.
EOF
)"
```

---

## Task 3: Add project-level template vars

**Files:**
- Create: `internal/projectenv/project_vars.go`
- Create: `internal/projectenv/project_vars_test.go`

`ProjectTemplateVars` takes the project name and TLD; produces `site_url`, `site_host`, `tls_cert_path`, `tls_key_path`. Cert/key paths come from `internal/certs/storage.go`.

- [ ] **Step 3.1: Write failing test**

Create `internal/projectenv/project_vars_test.go`:

```go
package projectenv

import (
	"strings"
	"testing"

	"github.com/prvious/pv/internal/certs"
)

func TestProjectTemplateVars_Defaults(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	got := ProjectTemplateVars("myapp", "test")

	if got["site_url"] != "https://myapp.test" {
		t.Errorf("site_url = %q, want %q", got["site_url"], "https://myapp.test")
	}
	if got["site_host"] != "myapp.test" {
		t.Errorf("site_host = %q, want %q", got["site_host"], "myapp.test")
	}
	if got["tls_cert_path"] != certs.CertPath("myapp.test") {
		t.Errorf("tls_cert_path = %q, want %q", got["tls_cert_path"], certs.CertPath("myapp.test"))
	}
	if got["tls_key_path"] != certs.KeyPath("myapp.test") {
		t.Errorf("tls_key_path = %q, want %q", got["tls_key_path"], certs.KeyPath("myapp.test"))
	}
	if !strings.HasSuffix(got["tls_cert_path"], "myapp.test.crt") {
		t.Errorf("tls_cert_path should end with myapp.test.crt, got %q", got["tls_cert_path"])
	}
}

func TestProjectTemplateVars_CustomTLD(t *testing.T) {
	t.Setenv("HOME", t.TempDir())

	got := ProjectTemplateVars("myapp", "dev")

	if got["site_url"] != "https://myapp.dev" {
		t.Errorf("site_url = %q, want %q", got["site_url"], "https://myapp.dev")
	}
	if got["site_host"] != "myapp.dev" {
		t.Errorf("site_host = %q, want %q", got["site_host"], "myapp.dev")
	}
}
```

- [ ] **Step 3.2: Run test, verify it fails**

Run: `go test ./internal/projectenv/ -run TestProjectTemplateVars -v`
Expected: FAIL — `undefined: ProjectTemplateVars`.

- [ ] **Step 3.3: Implement ProjectTemplateVars**

Create `internal/projectenv/project_vars.go`:

```go
package projectenv

import (
	"fmt"

	"github.com/prvious/pv/internal/certs"
)

// ProjectTemplateVars returns the template variables available at the
// top-level `env:` block in a pv.yml. projectName should already be
// sanitized by SanitizeProjectName; tld is the resolved per-machine
// TLD (e.g., "test").
func ProjectTemplateVars(projectName, tld string) map[string]string {
	host := fmt.Sprintf("%s.%s", projectName, tld)
	return map[string]string{
		"site_url":      "https://" + host,
		"site_host":     host,
		"tls_cert_path": certs.CertPath(host),
		"tls_key_path":  certs.KeyPath(host),
	}
}
```

- [ ] **Step 3.4: Verify**

Run: `go test ./internal/projectenv/ -v`
Expected: PASS.

- [ ] **Step 3.5: gofmt + vet + build**

Run: `gofmt -w internal/projectenv/ && go vet ./internal/projectenv/... && go build ./...`
Expected: no output.

- [ ] **Step 3.6: Commit**

```bash
git add internal/projectenv/project_vars.go internal/projectenv/project_vars_test.go
git commit -m "$(cat <<'EOF'
feat(projectenv): add project-level template vars

ProjectTemplateVars(projectName, tld) produces site_url, site_host,
tls_cert_path, tls_key_path for the top-level env: block in pv.yml.
Cert/key paths delegate to certs.CertPath / certs.KeyPath so the
storage layout stays single-sourced. Not yet wired into pv link.
EOF
)"
```

---

## Task 4: Postgres template vars

**Files:**
- Create: `internal/postgres/template_vars.go`
- Create: `internal/postgres/template_vars_test.go`

`TemplateVars` is the parallel of the existing `EnvVars` (`internal/postgres/envvars.go`) but produces the template-friendly map: `host`, `port`, `username`, `password`, `version`, `dsn`. No `database` — that's a project-level decision under pv.yml semantics (users call `pv postgres:db:create` from setup).

- [ ] **Step 4.1: Write failing test**

Create `internal/postgres/template_vars_test.go`:

```go
package postgres

import (
	"testing"
)

func TestTemplateVars_Major18(t *testing.T) {
	got, err := TemplateVars("18", "18.1")
	if err != nil {
		t.Fatalf("TemplateVars() error = %v", err)
	}
	if got["host"] != "127.0.0.1" {
		t.Errorf("host = %q, want 127.0.0.1", got["host"])
	}
	if got["port"] != "54018" {
		t.Errorf("port = %q, want 54018", got["port"])
	}
	if got["username"] != "postgres" {
		t.Errorf("username = %q, want postgres", got["username"])
	}
	if got["password"] != "postgres" {
		t.Errorf("password = %q, want postgres", got["password"])
	}
	if got["version"] != "18.1" {
		t.Errorf("version = %q, want 18.1", got["version"])
	}
	if got["dsn"] != "postgresql://postgres:postgres@127.0.0.1:54018" {
		t.Errorf("dsn = %q, want postgresql://postgres:postgres@127.0.0.1:54018", got["dsn"])
	}
}

func TestTemplateVars_InvalidMajorPropagates(t *testing.T) {
	_, err := TemplateVars("not-a-number", "")
	if err == nil {
		t.Fatal("TemplateVars() with bad major: want error, got nil")
	}
}
```

- [ ] **Step 4.2: Run test, verify it fails**

Run: `go test ./internal/postgres/ -run TestTemplateVars -v`
Expected: FAIL — `undefined: TemplateVars`.

- [ ] **Step 4.3: Implement TemplateVars**

Create `internal/postgres/template_vars.go`:

```go
package postgres

import (
	"fmt"
	"strconv"
)

// TemplateVars returns the variables available inside a pv.yml
// `postgresql.env:` block. The caller passes the major (e.g., "18")
// from pv.yml and the probed fullVersion (e.g., "18.1"); both come
// from outside so this function stays pure and testable.
func TemplateVars(major, fullVersion string) (map[string]string, error) {
	port, err := PortFor(major)
	if err != nil {
		return nil, err
	}
	const user = "postgres"
	const pass = "postgres"
	const host = "127.0.0.1"
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"username": user,
		"password": pass,
		"version":  fullVersion,
		"dsn":      fmt.Sprintf("postgresql://%s:%s@%s:%d", user, pass, host, port),
	}, nil
}
```

- [ ] **Step 4.4: Verify**

Run: `go test ./internal/postgres/ -v`
Expected: PASS.

- [ ] **Step 4.5: gofmt + vet + build**

Run: `gofmt -w internal/postgres/ && go vet ./internal/postgres/... && go build ./...`
Expected: no output.

- [ ] **Step 4.6: Commit**

```bash
git add internal/postgres/template_vars.go internal/postgres/template_vars_test.go
git commit -m "$(cat <<'EOF'
feat(postgres): add TemplateVars for pv.yml env templating

Returns host, port, username, password, version, dsn for the
postgresql.env block in pv.yml. Parallel of EnvVars but
template-friendly and database-less (db creation is now an explicit
user command, not a pv.yml side effect). Not yet wired.
EOF
)"
```

---

## Task 5: MySQL template vars

**Files:**
- Create: `internal/mysql/template_vars.go`
- Create: `internal/mysql/template_vars_test.go`

Mirror of Task 4 with MySQL's defaults (`root` username, empty password).

- [ ] **Step 5.1: Write failing test**

Create `internal/mysql/template_vars_test.go`:

```go
package mysql

import (
	"testing"
)

func TestTemplateVars_Version80(t *testing.T) {
	got, err := TemplateVars("8.0", "8.0.36")
	if err != nil {
		t.Fatalf("TemplateVars() error = %v", err)
	}
	if got["host"] != "127.0.0.1" {
		t.Errorf("host = %q, want 127.0.0.1", got["host"])
	}
	if got["port"] != "33080" {
		t.Errorf("port = %q, want 33080", got["port"])
	}
	if got["username"] != "root" {
		t.Errorf("username = %q, want root", got["username"])
	}
	if got["password"] != "" {
		t.Errorf("password = %q, want empty", got["password"])
	}
	if got["version"] != "8.0.36" {
		t.Errorf("version = %q, want 8.0.36", got["version"])
	}
	if got["dsn"] != "mysql://root:@127.0.0.1:33080" {
		t.Errorf("dsn = %q, want mysql://root:@127.0.0.1:33080", got["dsn"])
	}
}

func TestTemplateVars_InvalidVersionPropagates(t *testing.T) {
	_, err := TemplateVars("not-a-version", "")
	if err == nil {
		t.Fatal("TemplateVars() with bad version: want error, got nil")
	}
}
```

- [ ] **Step 5.2: Run test, verify it fails**

Run: `go test ./internal/mysql/ -run TestTemplateVars -v`
Expected: FAIL — `undefined: TemplateVars`.

- [ ] **Step 5.3: Implement TemplateVars**

Create `internal/mysql/template_vars.go`:

```go
package mysql

import (
	"fmt"
	"strconv"
)

// TemplateVars returns the variables available inside a pv.yml
// `mysql.env:` block. The caller passes the version (e.g., "8.0")
// from pv.yml and the probed fullVersion (e.g., "8.0.36"); both come
// from outside so this function stays pure and testable.
func TemplateVars(version, fullVersion string) (map[string]string, error) {
	port, err := PortFor(version)
	if err != nil {
		return nil, err
	}
	const user = "root"
	const pass = ""
	const host = "127.0.0.1"
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"username": user,
		"password": pass,
		"version":  fullVersion,
		"dsn":      fmt.Sprintf("mysql://%s:%s@%s:%d", user, pass, host, port),
	}, nil
}
```

- [ ] **Step 5.4: Verify**

Run: `go test ./internal/mysql/ -v`
Expected: PASS.

- [ ] **Step 5.5: gofmt + vet + build**

Run: `gofmt -w internal/mysql/ && go vet ./internal/mysql/... && go build ./...`
Expected: no output.

- [ ] **Step 5.6: Commit**

```bash
git add internal/mysql/template_vars.go internal/mysql/template_vars_test.go
git commit -m "$(cat <<'EOF'
feat(mysql): add TemplateVars for pv.yml env templating

Returns host, port, username (root), password (empty), version, dsn
for the mysql.env block in pv.yml. Mirrors postgres.TemplateVars.
Not yet wired.
EOF
)"
```

---

## Task 6: Redis template vars

**Files:**
- Create: `internal/redis/template_vars.go`
- Create: `internal/redis/template_vars_test.go`

Redis is single-version and param-less. Uses the existing `RedisPort` constant.

- [ ] **Step 6.1: Write failing test**

Create `internal/redis/template_vars_test.go`:

```go
package redis

import (
	"testing"
)

func TestTemplateVars(t *testing.T) {
	got := TemplateVars()

	if got["host"] != "127.0.0.1" {
		t.Errorf("host = %q, want 127.0.0.1", got["host"])
	}
	if got["port"] != "6379" {
		t.Errorf("port = %q, want 6379", got["port"])
	}
	if got["password"] != "" {
		t.Errorf("password = %q, want empty", got["password"])
	}
	if got["url"] != "redis://127.0.0.1:6379" {
		t.Errorf("url = %q, want redis://127.0.0.1:6379", got["url"])
	}
}
```

- [ ] **Step 6.2: Run test, verify it fails**

Run: `go test ./internal/redis/ -run TestTemplateVars -v`
Expected: FAIL — `undefined: TemplateVars`.

- [ ] **Step 6.3: Implement TemplateVars**

Create `internal/redis/template_vars.go`:

```go
package redis

import (
	"fmt"
	"strconv"
)

// TemplateVars returns the variables available inside a pv.yml
// `redis.env:` block. Redis is single-version with a fixed port, so
// no parameters are needed.
func TemplateVars() map[string]string {
	const host = "127.0.0.1"
	port := RedisPort
	return map[string]string{
		"host":     host,
		"port":     strconv.Itoa(port),
		"password": "",
		"url":      fmt.Sprintf("redis://%s:%d", host, port),
	}
}
```

- [ ] **Step 6.4: Verify**

Run: `go test ./internal/redis/ -v`
Expected: PASS.

- [ ] **Step 6.5: gofmt + vet + build**

Run: `gofmt -w internal/redis/ && go vet ./internal/redis/... && go build ./...`
Expected: no output.

- [ ] **Step 6.6: Commit**

```bash
git add internal/redis/template_vars.go internal/redis/template_vars_test.go
git commit -m "$(cat <<'EOF'
feat(redis): add TemplateVars for pv.yml env templating

Returns host, port (6379), password (empty), url for the redis.env
block in pv.yml. Param-less because redis is single-version.
Not yet wired.
EOF
)"
```

---

## Task 7: Mailpit template vars

**Files:**
- Create: `internal/mailpit/template_vars.go`
- Create: `internal/mailpit/template_vars_test.go`

Mailpit's SMTP and HTTP ports are fixed constants (1025 / 8025) embedded in `internal/mailpit/proc/proc.go`. The existing `internal/mailpit/service.go` already hardcodes `"1025"` as a string literal — we follow the same pattern.

- [ ] **Step 7.1: Write failing test**

Create `internal/mailpit/template_vars_test.go`:

```go
package mailpit

import (
	"testing"
)

func TestTemplateVars(t *testing.T) {
	got := TemplateVars()

	if got["smtp_host"] != "127.0.0.1" {
		t.Errorf("smtp_host = %q, want 127.0.0.1", got["smtp_host"])
	}
	if got["smtp_port"] != "1025" {
		t.Errorf("smtp_port = %q, want 1025", got["smtp_port"])
	}
	if got["http_host"] != "127.0.0.1" {
		t.Errorf("http_host = %q, want 127.0.0.1", got["http_host"])
	}
	if got["http_port"] != "8025" {
		t.Errorf("http_port = %q, want 8025", got["http_port"])
	}
}
```

- [ ] **Step 7.2: Run test, verify it fails**

Run: `go test ./internal/mailpit/ -run TestTemplateVars -v`
Expected: FAIL — `undefined: TemplateVars`.

- [ ] **Step 7.3: Implement TemplateVars**

Create `internal/mailpit/template_vars.go`:

```go
package mailpit

// TemplateVars returns the variables available inside a pv.yml
// `mailpit.env:` block. Mailpit is single-version with fixed ports
// (SMTP 1025, HTTP 8025) — values match what the existing service
// layer publishes for the running process.
func TemplateVars() map[string]string {
	return map[string]string{
		"smtp_host": "127.0.0.1",
		"smtp_port": "1025",
		"http_host": "127.0.0.1",
		"http_port": "8025",
	}
}
```

- [ ] **Step 7.4: Verify**

Run: `go test ./internal/mailpit/ -v`
Expected: PASS.

- [ ] **Step 7.5: gofmt + vet + build**

Run: `gofmt -w internal/mailpit/ && go vet ./internal/mailpit/... && go build ./...`
Expected: no output.

- [ ] **Step 7.6: Commit**

```bash
git add internal/mailpit/template_vars.go internal/mailpit/template_vars_test.go
git commit -m "$(cat <<'EOF'
feat(mailpit): add TemplateVars for pv.yml env templating

Returns smtp_host, smtp_port (1025), http_host, http_port (8025) for
the mailpit.env block in pv.yml. Param-less; ports match existing
service layer constants. Not yet wired.
EOF
)"
```

---

## Task 8: RustFS template vars

**Files:**
- Create: `internal/rustfs/template_vars.go`
- Create: `internal/rustfs/template_vars_test.go`

RustFS port is 9000 (also a fixed constant in `internal/rustfs/proc/proc.go`). Access/secret key default to `rstfsadmin` per the existing `service.go` `EnvVars`.

- [ ] **Step 8.1: Write failing test**

Create `internal/rustfs/template_vars_test.go`:

```go
package rustfs

import (
	"testing"
)

func TestTemplateVars(t *testing.T) {
	got := TemplateVars()

	if got["endpoint"] != "http://127.0.0.1:9000" {
		t.Errorf("endpoint = %q, want http://127.0.0.1:9000", got["endpoint"])
	}
	if got["access_key"] != "rstfsadmin" {
		t.Errorf("access_key = %q, want rstfsadmin", got["access_key"])
	}
	if got["secret_key"] != "rstfsadmin" {
		t.Errorf("secret_key = %q, want rstfsadmin", got["secret_key"])
	}
	if got["region"] != "us-east-1" {
		t.Errorf("region = %q, want us-east-1", got["region"])
	}
	if got["use_path_style"] != "true" {
		t.Errorf("use_path_style = %q, want true", got["use_path_style"])
	}
}
```

- [ ] **Step 8.2: Run test, verify it fails**

Run: `go test ./internal/rustfs/ -run TestTemplateVars -v`
Expected: FAIL — `undefined: TemplateVars`.

- [ ] **Step 8.3: Implement TemplateVars**

Create `internal/rustfs/template_vars.go`:

```go
package rustfs

// TemplateVars returns the variables available inside a pv.yml
// `rustfs.env:` block. Values mirror the existing service.EnvVars
// defaults (admin/admin credentials, us-east-1, path-style addressing)
// minus the project-name-derived bucket — bucket creation is now an
// explicit user command, not a pv.yml side effect.
func TemplateVars() map[string]string {
	return map[string]string{
		"endpoint":       "http://127.0.0.1:9000",
		"access_key":     "rstfsadmin",
		"secret_key":     "rstfsadmin",
		"region":         "us-east-1",
		"use_path_style": "true",
	}
}
```

- [ ] **Step 8.4: Verify**

Run: `go test ./internal/rustfs/ -v`
Expected: PASS.

- [ ] **Step 8.5: gofmt + vet + build**

Run: `gofmt -w internal/rustfs/ && go vet ./internal/rustfs/... && go build ./...`
Expected: no output.

- [ ] **Step 8.6: Commit**

```bash
git add internal/rustfs/template_vars.go internal/rustfs/template_vars_test.go
git commit -m "$(cat <<'EOF'
feat(rustfs): add TemplateVars for pv.yml env templating

Returns endpoint, access_key, secret_key, region, use_path_style for
the rustfs.env block in pv.yml. Bucket name is omitted because
bucket creation is now an explicit user command. Not yet wired.
EOF
)"
```

---

## Task 9: Final verification

**Files:** none modified.

This task is a sanity sweep — make sure the full tree builds, all tests pass, and `pv` itself still behaves identically (since this PR is parse-only). No commit unless something is wrong.

- [ ] **Step 9.1: Format check (project rule)**

Run: `gofmt -l .`
Expected: no output (no unformatted files).

- [ ] **Step 9.2: Vet sweep**

Run: `go vet ./...`
Expected: no output.

- [ ] **Step 9.3: Full test sweep**

Run: `go test ./...`
Expected: PASS for every package.

- [ ] **Step 9.4: Full build**

Run: `go build ./...`
Expected: no output, no errors.

- [ ] **Step 9.5: Behaviour smoke — `pv --version` and `pv link --help` still work**

Run:
```bash
go build -o /tmp/pv-pr1-smoke . && /tmp/pv-pr1-smoke --version && /tmp/pv-pr1-smoke link --help
rm /tmp/pv-pr1-smoke
```
Expected: version line prints; link help prints. No errors. This confirms no command was accidentally broken by struct changes (yaml unmarshalling new fields shouldn't affect anything, but verify).

- [ ] **Step 9.6: Confirm PR is shippable**

Last commit on the branch should be Task 8's `feat(rustfs): …`. There should be no uncommitted changes. The branch can be opened as a PR titled along the lines of `feat(config): pv.yml schema + template engine (PR 1/6)`.

Run: `git status && git log --oneline -8`
Expected: working tree clean; commits 1–8 visible in order.

---

## Self-Review (already applied)

**Spec coverage:**
- ✅ Schema fields (aliases, env, services, setup) — Task 1
- ✅ ServiceConfig with version + env — Task 1
- ✅ Template renderer with missingkey=error — Task 2
- ✅ Project-level template vars (site_url, site_host, tls_cert_path, tls_key_path) — Task 3
- ✅ Per-service template var producers (postgres, mysql, redis, mailpit, rustfs) — Tasks 4–8
- ✅ No runtime behavior change in `pv link` / `pv setup` — none of these tasks modify those callers
- ✅ Each test asserts every variable named in the spec's template var tables

**Placeholders:** none — every step has the exact code to write and the exact command to run.

**Type consistency:**
- `Render(string, map[string]string) (string, error)` — same shape used by all tests
- `TemplateVars` returns `(map[string]string, error)` for postgres/mysql (because PortFor returns error) and `map[string]string` for redis/mailpit/rustfs (constant ports). Tests match.
- `*ServiceConfig` pointer fields — tests check for `nil` consistently when undeclared.

**Deliberate deviations from the spec, called out so future readers don't think they're omissions:**
- Postgres/MySQL `TemplateVars` accepts the `fullVersion` string as a parameter rather than probing the binary internally. This keeps the function pure and unit-testable; PR 2's caller invokes `postgres.ProbeVersion(major)` / `mysql.ProbeVersion(version)` and passes the result in. This is consistent with the spec because the spec defines the template variable *value*, not the function signature.

