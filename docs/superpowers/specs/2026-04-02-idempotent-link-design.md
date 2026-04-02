# Idempotent `pv link`

**Date:** 2026-04-02
**Status:** Approved

## Problem

Running `pv link` on an already-linked project errors with "already linked, run unlink then link". This is annoying for the common case of re-linking a project to refresh its config (e.g., after changing PHP version, project type, or moving directories).

## Design

### Approach

Make `pv link` idempotent. If the project name is already registered, update it in place instead of erroring. Preserve service bindings and databases from the existing entry; re-detect type and PHP version.

### Changes

#### 1. Registry: add `Update` method (`internal/registry/registry.go`)

New method that replaces an existing project entry by name:

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

#### 2. Link command: update path (`cmd/link.go`)

Replace the "already linked" error with an update flow:

1. When `reg.Find(name)` finds an existing project:
   - Save `Services` and `Databases` from the existing entry
   - Re-detect `Type` and resolve `PHP` version (same logic as fresh link)
   - Build new `Project` with preserved `Services`/`Databases` + fresh `Path`/`Type`/`PHP`
   - Remove old Caddy site config and TLS cert (cleanup before pipeline regenerates)
   - Call `reg.Update(project)` then `reg.Save()`
2. When not found: proceed as today with `reg.Add(project)` then `reg.Save()`
3. The full automation pipeline runs in both cases — same steps, same order
4. Success output says "Relinked" instead of "Linked" when updating an existing entry

#### 3. No changes required

- **Automation pipeline** — same steps run for both fresh and re-link
- **`cmd/unlink.go`** — unchanged
- **Caddy/certs/DNS** — pipeline handles regeneration
- **Registry `Add`/`Remove`** — unchanged, `Update` is additive

### Edge cases

- **Name collision with different path**: `pv link --name=myapp ~/new-path` when `myapp` is linked to `~/old-path` — updates the path. User explicitly chose the name. Old configs cleaned up, new ones generated.
- **Same path, same name**: Pure refresh — re-detects type/PHP, regenerates configs, re-runs automation.
- **Services/databases preserved**: Existing service bindings (MySQL, Redis, etc.) and database list carry over to the updated entry.

### Testing

- Add unit test for `Registry.Update()` — updates existing entry, returns error for non-existent
- Add test in `cmd/` that links, then re-links the same project — verify no error, verify services preserved
- Existing link/unlink tests should continue to pass unchanged
