# pv.yml Managed Env Labels Design

**Date:** 2026-05-12
**Status:** Proposed
**Target:** PR 6 of the pv.yml rollout

## Goal

Make `.env` keys written from `pv.yml` visibly identifiable without introducing hidden state and without deleting keys that disappear from `pv.yml`.

PR 5 made `pv.yml` mandatory and removed the legacy silent env writers. PR 6 should now make pv's remaining explicit env writes transparent to users: if pv writes a key from `pv.yml`, the `.env` file should show that the key was pv-managed.

## Non-Goals

- Do not remove stale `.env` keys when a key disappears from `pv.yml`.
- Do not create a separate state file for managed keys.
- Do not group managed keys into a dedicated block.
- Do not add commands for listing or cleaning managed keys.
- Do not change the `pv.yml` schema.

## User Model

`pv.yml` is the source of truth only while a key remains declared there. Removing a key from `pv.yml` is like unsubscribing from pv management for that key: pv stops updating it, and the user can edit or remove the existing `.env` entry manually.

Example before relink:

```yaml
env:
  APP_URL: "{{ .site_url }}"
postgresql:
  version: "18"
  env:
    DB_HOST: "{{ .host }}"
```

Resulting `.env`:

```dotenv
# pv-managed
APP_URL=https://myapp.test

# pv-managed
DB_HOST=127.0.0.1

CUSTOM_THING=keep-me
```

If the user later removes `APP_URL` from `pv.yml`, `pv link` leaves the existing `.env` line alone:

```dotenv
# pv-managed
APP_URL=https://myapp.test

# pv-managed
DB_HOST=127.0.0.1

CUSTOM_THING=keep-me
```

From that point on, pv no longer updates `APP_URL`; the user owns cleanup.

## Marker Format

Use an adjacent comment immediately above each key pv writes:

```dotenv
# pv-managed
KEY=value
```

Do not use inline comments like `KEY=value # pv-managed`. Dotenv parsers differ on whether inline comments are stripped or treated as part of the value. A preceding full-line comment is safer and readable.

## Merge Behavior

`ApplyPvYmlEnvStep` currently renders all top-level and per-service `env:` declarations into a `map[string]string`, then calls `projectenv.MergeDotEnv`. PR 6 changes that write path to label rendered keys as pv-managed.

The merge rules are:

1. When pv appends a new rendered key, write `# pv-managed` immediately before `KEY=value`.
2. When pv updates an existing rendered key, ensure the previous line is `# pv-managed`.
3. If the existing key already has the marker immediately above it, do not duplicate the marker.
4. If an existing rendered key has comments above it, preserve those comments and insert `# pv-managed` directly above the key unless the marker is already there.
5. If a key is not in the newly rendered pv.yml env set, leave it untouched even if it has an old `# pv-managed` marker.
6. Keep the existing `.pv-backup` behavior before modifying `.env`.

This preserves the user's file layout as much as possible while making pv's writes explicit.

## File Responsibilities

- `internal/projectenv/dotenv.go` owns `.env` parsing and merging. Add the managed-label merge helper here so dotenv formatting concerns stay in one package.
- `internal/automation/steps/apply_pvyml_env.go` owns rendering pv.yml env templates. It should call the managed-label helper instead of the generic merge helper for pv.yml writes.
- `internal/laravel/env.go` still uses the generic `MergeDotEnv` for uninstall fallback hooks. Those fallback writes are not pv.yml-managed declarations and should not receive `# pv-managed` labels in this PR.

## Error Handling

Use the existing error model:

- File read/write errors bubble up from `projectenv`.
- `ApplyPvYmlEnvStep` wraps merge failures as `merge .env: ...`.
- Backup write failure remains fatal, matching current `MergeDotEnv` behavior.

## Testing

Add tests around the projectenv merge helper and the automation step integration.

Required behavior tests:

- Appending new pv.yml env keys writes `# pv-managed` before each key.
- Updating an existing key adds the marker when missing.
- Updating an already-marked key does not duplicate `# pv-managed`.
- Removing a key from `pv.yml` does not remove the existing `.env` key or marker.
- Non-pv keys remain untouched.
- `.pv-backup` is still written before modifications.

The tests should avoid relying on map iteration order where possible. If testing appended output, use a single rendered key or assert substrings instead of exact full-file order.

## Documentation

Update the README migration/env section to say pv labels env keys it writes with `# pv-managed`, and that removing a key from `pv.yml` stops future updates but does not delete the existing `.env` line.

Update the original pv.yml explicit config spec's PR 6 section to replace managed-key cleanup with managed-labeling behavior.

## Acceptance Criteria

- `pv link` writes pv.yml-rendered env keys with adjacent `# pv-managed` comments.
- `pv link` keeps updating currently declared pv.yml env keys.
- Removing a key from `pv.yml` leaves the existing `.env` entry untouched.
- No separate managed-key state file is created.
- Existing non-pv `.env` keys are not marked, edited, or removed.
- `go test ./...`, `go vet ./...`, and `go build ./...` pass.
