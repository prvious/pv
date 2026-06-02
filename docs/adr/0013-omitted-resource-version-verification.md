# ADR 0013: Omitted Resource Version Verification

## Status

Accepted

## Context

Project config Managed Resource blocks can omit `version`. PV needed focused
verification that omitted versions use the same contract as explicit `latest`:
they resolve to the artifact manifest default concrete track before desired
state is written, and existing Projects keep their stored concrete track when a
manifest default changes later.

Related reviewed PRs:

- Test/docs coverage: https://github.com/prvious/pv/pull/247
- Repair coverage: https://github.com/prvious/pv/pull/248

## Decision

PV treats an omitted Project config Managed Resource `version` as the `latest`
selector. `latest` resolves to the Managed Resource manifest `default_track`.
Reconciliation persists only the resolved concrete track. Project config is not
rewritten, and existing Project state does not float when manifest defaults
change.

The responsibility split remains:

- `config` parses omitted resource versions as `ResourceConfig.track: None`.
- `daemon` resolves omitted and explicit `latest` selectors before state writes.
- `resources` owns manifest default-track selection.
- `state` rejects reserved alias tracks such as `latest`.

## Verification

Final focused verification ran against the reviewed omitted-version coverage
branch:

```shell
cargo nextest run -p daemon --test project_env_reconciliation -E 'test(omitted_resource_track_resolves_manifest_default_track) or test(omitted_resource_track_reuses_stored_track_when_manifest_default_changes) or test(latest_resource_track_resolves_default_track_before_state_and_dotenv_writes) or test(latest_resource_track_reuses_stored_track_when_manifest_default_changes)'
cargo nextest run -p state -E 'test(resource_state_apis_reject_latest_tracks)'
git diff --check
cargo fmt --all -- --check
```

Results:

- Daemon focused nextest: 4 run, 4 passed, 18 skipped.
- State focused nextest: 1 run, 1 passed, 51 skipped.
- `git diff --check`: exit 0.
- `cargo fmt --all -- --check`: exit 0.

Solo scoped TODOs for `feature-omitted-resource-version` were verified with
owner and scope tags during the review handoff.
