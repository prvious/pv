# Idempotent `pv link` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `pv link` idempotent — re-linking an already-linked project updates it in place (preserving services/databases) instead of erroring.

**Architecture:** Add `Registry.Update()` method, then replace the "already linked" error in `cmd/link.go` with an update-in-place flow that preserves service bindings and re-runs the automation pipeline.

**Tech Stack:** Go, cobra CLI

**Spec:** `docs/superpowers/specs/2026-04-02-idempotent-link-design.md`

---

### Task 1: Add `Registry.Update()` method with tests

**Files:**
- Modify: `internal/registry/registry.go:71-77`
- Modify: `internal/registry/registry_test.go`

- [ ] **Step 1: Write failing tests for `Update`**

Add these tests after the existing `TestRemove_Last` test in `internal/registry/registry_test.go`:

```go
func TestUpdate_Existing(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "foo", Path: "/tmp/foo", Type: "php"})

	err := r.Update(Project{Name: "foo", Path: "/tmp/foo2", Type: "laravel"})
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if len(r.Projects) != 1 {
		t.Fatalf("expected 1 project, got %d", len(r.Projects))
	}
	if r.Projects[0].Path != "/tmp/foo2" {
		t.Errorf("expected path %q, got %q", "/tmp/foo2", r.Projects[0].Path)
	}
	if r.Projects[0].Type != "laravel" {
		t.Errorf("expected type %q, got %q", "laravel", r.Projects[0].Type)
	}
}

func TestUpdate_NonExistent(t *testing.T) {
	r := &Registry{}
	err := r.Update(Project{Name: "foo", Path: "/tmp/foo"})
	if err == nil {
		t.Fatal("expected error for non-existent project, got nil")
	}
}

func TestUpdate_PreservesOrder(t *testing.T) {
	r := &Registry{}
	_ = r.Add(Project{Name: "a", Path: "/tmp/a"})
	_ = r.Add(Project{Name: "b", Path: "/tmp/b"})
	_ = r.Add(Project{Name: "c", Path: "/tmp/c"})

	err := r.Update(Project{Name: "b", Path: "/tmp/b2", Type: "laravel"})
	if err != nil {
		t.Fatalf("Update() error = %v", err)
	}
	if r.Projects[0].Name != "a" || r.Projects[1].Name != "b" || r.Projects[2].Name != "c" {
		t.Errorf("expected order [a b c], got [%s %s %s]", r.Projects[0].Name, r.Projects[1].Name, r.Projects[2].Name)
	}
	if r.Projects[1].Path != "/tmp/b2" {
		t.Errorf("expected updated path, got %q", r.Projects[1].Path)
	}
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `go test ./internal/registry/ -run "TestUpdate" -v`
Expected: FAIL — `Update` method not defined.

- [ ] **Step 3: Implement `Update` method**

In `internal/registry/registry.go`, add this method after the `Add` method (after line 77):

```go
func (r *Registry) Update(p Project) error {
	for i, existing := range r.Projects {
		if existing.Name == p.Name {
			r.Projects[i] = p
			return nil
		}
	}
	return fmt.Errorf("project %q not found", p.Name)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `go test ./internal/registry/ -v`
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add internal/registry/registry.go internal/registry/registry_test.go
git commit -m "Add Registry.Update() method for in-place project updates

Replaces an existing project entry by name, preserving array order.
Returns error if project not found."
```

### Task 2: Make `pv link` idempotent

**Files:**
- Modify: `cmd/link.go:64-67`

- [ ] **Step 1: Write failing test for re-link behavior**

In `cmd/link_test.go`, replace the existing `TestLink_DuplicateName` test (lines 99-117) with:

```go
func TestLink_RelinkPreservesServices(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeDefaultSettings(t)

	projDir := t.TempDir()

	// First link.
	cmd1 := newLinkCmd()
	cmd1.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd1.Execute(); err != nil {
		t.Fatalf("first link error = %v", err)
	}

	// Manually add services to the registry entry to simulate bound services.
	reg, err := registry.Load()
	if err != nil {
		t.Fatalf("load registry: %v", err)
	}
	p := reg.Find("myapp")
	if p == nil {
		t.Fatal("expected project myapp in registry")
	}
	p.Services = &registry.ProjectServices{MySQL: "mysql:8.0"}
	p.Databases = []string{"myapp"}
	if err := reg.Save(); err != nil {
		t.Fatalf("save registry: %v", err)
	}

	// Re-link the same project — should succeed, not error.
	cmd2 := newLinkCmd()
	cmd2.SetArgs([]string{"link", projDir, "--name", "myapp"})
	if err := cmd2.Execute(); err != nil {
		t.Fatalf("re-link should succeed, got error: %v", err)
	}

	// Verify services and databases were preserved.
	reg2, err := registry.Load()
	if err != nil {
		t.Fatalf("load registry after relink: %v", err)
	}
	p2 := reg2.Find("myapp")
	if p2 == nil {
		t.Fatal("expected project myapp in registry after relink")
	}
	if p2.Services == nil || p2.Services.MySQL != "mysql:8.0" {
		t.Errorf("expected MySQL service preserved, got services=%v", p2.Services)
	}
	if len(p2.Databases) != 1 || p2.Databases[0] != "myapp" {
		t.Errorf("expected databases preserved, got %v", p2.Databases)
	}
}
```

Note: You need to check what `ProjectServices` fields are available. Read `internal/registry/registry.go` for the struct definition.

- [ ] **Step 2: Run test to verify it fails**

Run: `go test ./cmd/ -run TestLink_RelinkPreservesServices -v`
Expected: FAIL — re-link still errors with "already linked".

- [ ] **Step 3: Update `cmd/link.go` to handle re-link**

In `cmd/link.go`, replace the existing duplicate-check block (lines 65-67):

```go
		if existing := reg.Find(name); existing != nil {
			return fmt.Errorf("%s is already linked at %s\nTo re-link, run: pv unlink %s && pv link %s", name, existing.Path, name, path)
		}
```

with:

```go
		// Check if project is already linked — if so, update in place.
		var relink bool
		var preservedServices *registry.ProjectServices
		var preservedDatabases []string
		if existing := reg.Find(name); existing != nil {
			relink = true
			preservedServices = existing.Services
			preservedDatabases = existing.Databases

			// Clean up old configs before pipeline regenerates them.
			_ = caddy.RemoveSiteConfig(name)
			hostname := name + "." + settings.Defaults.TLD
			_ = certs.RemoveSiteTLS(hostname)
		}
```

You will need to add `"github.com/prvious/pv/internal/caddy"` and `"github.com/prvious/pv/internal/certs"` to the imports (if not already present).

Then, move the settings load above this block (it currently happens at line 71-75, but we need `settings.Defaults.TLD` for the cert cleanup). Read the current file to find the exact placement.

- [ ] **Step 4: Update the registry operation**

Replace the current `reg.Add(project)` call (around line 84) with:

```go
		// Register or update project.
		project := registry.Project{Name: name, Path: absPath, Type: projectType, PHP: phpVersion}
		if relink {
			project.Services = preservedServices
			project.Databases = preservedDatabases
			if err := reg.Update(project); err != nil {
				return err
			}
		} else {
			if err := reg.Add(project); err != nil {
				return err
			}
		}
```

- [ ] **Step 5: Update success output**

Replace the success output line that prints "Linked" (around line 135):

```go
		action := "Linked"
		if relink {
			action = "Relinked"
		}

		fmt.Fprintln(os.Stderr)
		ui.Success(fmt.Sprintf("%s %s", action, ui.Accent.Bold(true).Render(domain)))
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `go test ./cmd/ -run TestLink -v`
Expected: All PASS, including the new `TestLink_RelinkPreservesServices`.

- [ ] **Step 7: Run full test suite**

Run: `go test ./...`
Expected: All PASS.

- [ ] **Step 8: Run lint checks**

Run: `gofmt -l . && go vet ./...`
Expected: Clean.

- [ ] **Step 9: Commit**

```bash
git add cmd/link.go cmd/link_test.go
git commit -m "Make pv link idempotent — update in place if already linked

Re-linking preserves service bindings and databases, re-detects
project type and PHP version, and re-runs the full automation
pipeline. Shows 'Relinked' in success output."
```
