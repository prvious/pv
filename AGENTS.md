# AGENTS.md

Guide for AI coding agents working in the `pv` codebase.

## What is pv

`pv` is a local development server manager powered by FrankenPHP (Caddy + embedded PHP). It manages FrankenPHP instances serving projects under `.test` domains with HTTPS, supporting multiple PHP versions simultaneously. Written in Go using Cobra for CLI.

## Build, Test & Lint Commands

```bash
# Build
go build -o pv .

# Run all tests
go test ./...

# Run tests for a single package
go test ./internal/registry/
go test ./cmd/

# Run a single test or matching pattern
go test ./cmd/ -run TestLink
go test ./internal/phpenv/ -run TestResolveVersion

# Verbose output
go test ./... -v

# Test with coverage
go test ./... -cover

# Format code (use goimports, not gofmt)
goimports -w .

# Lint (if golangci-lint is available)
golangci-lint run
```

## Architecture Overview

```
main.go                 # Entry point → calls cmd.Execute()
cmd/                    # Cobra commands (user-facing CLI)
internal/
  config/               # ~/.pv/ paths & settings
  registry/             # Project registry (JSON)
  phpenv/               # PHP version management
  caddy/                # Caddyfile generation
  server/               # Process management (FrankenPHP + DNS)
  binaries/             # Binary downloads
  detection/            # Project type detection
  setup/                # Installation helpers
```

See `CLAUDE.md` for detailed architecture, directory layout, and multi-version architecture.

## Code Style Guidelines

### Imports

Use standard Go import order (automatically handled by `goimports`):
```go
import (
    // 1. Standard library (alphabetical)
    "encoding/json"
    "fmt"
    "os"
    
    // 2. External packages (alphabetical)
    "github.com/spf13/cobra"
    
    // 3. Internal packages (alphabetical)
    "github.com/prvious/pv/internal/config"
    "github.com/prvious/pv/internal/registry"
)
```

### Formatting

- Use `goimports` (not `gofmt`) — it handles imports + formatting
- Tabs for indentation (Go standard)
- No trailing whitespace
- One declaration per line

### Types

**Struct definitions:**
```go
// JSON-serializable structs use tags
type Project struct {
    Name string `json:"name"`
    Path string `json:"path"`
    Type string `json:"type"`
    PHP  string `json:"php,omitempty"`  // omitempty for optional
}

// Internal structs (no serialization) use simple form
type siteData struct {
    Name     string
    Path     string
    RootPath string
}
```

**Always use pointer receivers for methods:**
```go
func (r *Registry) Add(p Project) error { ... }
func (s *Settings) Save() error { ... }
```

### Naming Conventions

**Variables:**
- Short names in local scope: `reg`, `p`, `s`, `v`, `err`
- Full names for package-level/exported: `linkName`, `Settings`, `GlobalVersion`
- Single-letter or short receivers: `r` for Registry, `s` for Settings

**Functions:**
- Action verbs: `Add`, `Remove`, `Save`, `Start`, `Stop`, `Install`
- Query verbs: `Find`, `List`, `IsInstalled`, `IsRunning`
- Get/Set: `GlobalVersion`, `SetGlobal`
- Generate: `GenerateSiteConfig`, `GenerateCaddyfile`
- Resolve: `ResolveVersion`, `resolveRoot`

**Tests:**
```go
// Format: Test{FunctionName}_{Scenario}
func TestAdd_ToEmpty(t *testing.T) { ... }
func TestAdd_Duplicate(t *testing.T) { ... }
func TestRemove_NonExistent(t *testing.T) { ... }
```

**Constants:**
- UPPER_SNAKE_CASE for config: `DNSPort = 10053`
- camelCase for templates (unexported): `laravelTmpl`, `mainCaddyfile`

### Error Handling

**Always return errors as last value:**
```go
func Load() (*Registry, error) { ... }
func (r *Registry) Save() error { ... }
```

**Wrap errors with context using fmt.Errorf + %w:**
```go
if err := registry.Load(); err != nil {
    return fmt.Errorf("cannot load registry: %w", err)
}
```

**Create new errors with fmt.Errorf (no %w):**
```go
if name == "" {
    return fmt.Errorf("project name cannot be empty")
}
```

**Check errors immediately:**
```go
data, err := os.ReadFile(path)
if err != nil {
    if os.IsNotExist(err) {
        return &Registry{}, nil  // Special case first
    }
    return nil, err  // General error
}
```

**No naked returns — always explicit:**
```go
if err != nil {
    return nil, err  // Explicit nil, explicit error
}
return &reg, nil  // Explicit value, explicit nil
```

### Comments

**Godoc style for exported functions:**
```go
// InstalledVersions returns all PHP versions that have been installed.
// It scans ~/.pv/php/ for directories containing a frankenphp binary.
func InstalledVersions() ([]string, error) { ... }
```

- First sentence is summary (appears in godoc)
- Explain parameters, return values, and special cases
- Full sentences with periods for godoc comments
- No period for short inline comments

### Testing Patterns

**CRITICAL: Always isolate tests with t.TempDir() + t.Setenv:**
```go
func TestSomething(t *testing.T) {
    home := t.TempDir()
    t.Setenv("HOME", home)
    // All ~/.pv/ operations now go to temp dir
}
```

**Helper functions must use t.Helper():**
```go
func scaffold(t *testing.T) string {
    t.Helper()  // Makes failures point to caller
    home := t.TempDir()
    t.Setenv("HOME", home)
    return home
}
```

**Build fresh cobra commands per test:**
```go
func newLinkCmd() *cobra.Command {
    var name string  // Local variable
    root := &cobra.Command{Use: "pv"}
    link := &cobra.Command{
        Use:  "link",
        RunE: func(cmd *cobra.Command, args []string) error {
            linkName = name  // Sync to package var
            return linkCmd.RunE(cmd, args)
        },
    }
    link.Flags().StringVar(&name, "name", "", "")
    root.AddCommand(link)
    return root
}
```

**Table-driven tests for multiple cases:**
```go
func TestPortForVersion(t *testing.T) {
    tests := []struct {
        version string
        want    int
    }{
        {"8.3", 8830},
        {"8.4", 8840},
    }
    for _, tt := range tests {
        t.Run(tt.version, func(t *testing.T) {
            got := PortForVersion(tt.version)
            if got != tt.want {
                t.Errorf("got %d, want %d", got, tt.want)
            }
        })
    }
}
```

**Standard assertions:**
```go
if err != nil {
    t.Fatalf("Function() error = %v", err)  // Fatal stops
}
if got != want {
    t.Errorf("got %q, want %q", got, want)  // Error continues
}
```

### File Operations

**Always use filepath package:**
```go
path := filepath.Join(config.SitesDir(), name+".caddy")  // NOT string concat
name := filepath.Base(absPath)
dir := filepath.Dir(destPath)
```

**Standard permissions:**
```go
os.WriteFile(path, data, 0644)   // Regular files
os.MkdirAll(dir, 0755)           // Directories  
os.Chmod(path, 0755)             // Executables
```

**Atomic file writes (temp + rename):**
```go
tmp, err := os.CreateTemp(dir, ".pv-download-*")
// ... write to tmp ...
if err := tmp.Close(); err != nil {
    os.Remove(tmp.Name())
    return err
}
if err := os.Rename(tmp.Name(), destPath); err != nil {
    os.Remove(tmp.Name())
    return err
}
```

## Key Principles

1. **Test isolation via HOME redirection** — `t.Setenv("HOME", t.TempDir())`
2. **Fresh cobra commands for tests** — Avoid state leakage
3. **Error wrapping with context** — `fmt.Errorf("...: %w", err)`
4. **No interfaces** — All concrete types, no mocking
5. **Helper functions marked with t.Helper()** — Better error messages
6. **Atomic file operations** — temp file + rename
7. **Pointer receivers everywhere** — Consistency
8. **Standard library first** — Minimal external dependencies
9. **Explicit returns** — No naked returns
10. **Use goimports, not gofmt** — Handles imports + formatting

## Testing Strategy

- **Unit tests** (`go test ./...`): Run locally with filesystem isolation via `t.Setenv("HOME", t.TempDir())`. Use fake binaries (bash scripts) when needed.
- **E2E tests** (`.github/workflows/e2e.yml` + `scripts/e2e/`): Run on GitHub Actions for real binary execution, network calls, DNS, HTTPS. Add scripts to `scripts/e2e/` for integration scenarios.

When your feature needs real PHP/Composer/FrankenPHP/DNS/HTTPS, create an E2E test script.
