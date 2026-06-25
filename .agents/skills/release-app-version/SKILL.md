---
name: release-app-version
description: Build, publish, and verify PV application releases. Use when the user asks to bump the PV app version, run the PV App Release workflow, publish app binaries, update pv-app-manifest.json or install.sh, inspect app release workflow inputs, or release a new stable PV CLI version.
---

# Release App Version

Requested operation: $ARGUMENTS

Use this workflow for PV application releases. This is separate from Managed Resource artifact releases: app releases publish native `pv` binaries, `pv-app-manifest.json`, and `install.sh`.

## Core Rules

- The app release version comes from root `Cargo.toml`.
- Keep workspace package versions in sync unless the user explicitly asks for a narrower change.
- App publication must use a successful `PV App Release` workflow run from the same commit as the `PV App Publication` run.
- Do not dispatch release or publication workflows until you inspect the current workflow inputs and confirm exact values with the user.

## Before Editing Or Dispatching

1. Read the repo rules and release design:

   ```sh
   sed -n '1,180p' CONTRIBUTING.md
   sed -n '72,84p' DESIGN.md
   sed -n '409,580p' DESIGN.md
   git status --short --branch
   ```

2. Inspect app version fields:

   ```sh
   rg -n '^version = ' Cargo.toml crates/*/Cargo.toml
   rg -n 'env!\("CARGO_PKG_VERSION"\)|PV_DEFAULT_APP_UPDATE_MANIFEST_URL|pv-app-manifest' crates .github Cargo.toml
   ```

3. Inspect available workflow inputs before choosing values:

   ```sh
   sed -n '1,300p' .github/workflows/app-release.yml
   sed -n '1,340p' .github/workflows/app-publication.yml
   ```

   If local files may not match the dispatch ref, inspect the remote workflow with `gh workflow view`.

4. Confirm exact workflow inputs with the user before dispatching:
   - `PV App Release`: git ref, `minimum_pv_version`, `app_platforms`
   - `PV App Publication`: git ref, `source_run_id`
   - whether this is build-only or build-and-publish

Do not silently rely on defaults. Current checked-in defaults are `minimum_pv_version=0.1.0` and `app_platforms=darwin-arm64`, but always re-read the workflow first.

## Bump The App Version

Update the root package and workspace package versions together unless the release is intentionally different:

```txt
Cargo.toml
crates/*/Cargo.toml
Cargo.lock
```

Use precise lockfile updates for every changed workspace package. Do not run a broad dependency update.

```sh
cargo update -p pv --precise <new-version>
cargo update -p cli --precise <new-version>
cargo update -p config --precise <new-version>
cargo update -p daemon --precise <new-version>
cargo update -p platform --precise <new-version>
cargo update -p protocol --precise <new-version>
cargo update -p pv-release --precise <new-version>
cargo update -p resources --precise <new-version>
cargo update -p self-update --precise <new-version>
cargo update -p state --precise <new-version>
```

If the app release does not require code changes, keep the commit limited to version files and expected snapshots.

## Local Verification

Focused release checks:

```sh
cargo nextest run -p pv-release --test app_release_records
cargo nextest run -p pv-release --test app_publication
cargo nextest run -p pv-release --test workflow_defaults
cargo nextest run -p self-update --test app_update_manifest
cargo nextest run -p cli --test update
```

If snapshots change, inspect them first and accept only expected changes:

```sh
cargo insta accept --all
```

Final verification before commit, push, workflow dispatch, publication, or any claim that the release prep is ready:

```sh
cargo fmt --all --check
cargo insta pending-snapshots --workspace
git diff --check
cargo nextest run --workspace --all-features --locked --no-fail-fast
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Focused release checks do not replace the full workspace suite. If the full suite produces expected snapshot drift, inspect the generated snapshots, accept them only through `cargo insta`, then rerun the full workspace suite until it passes.

## Commit And Push

Stage only release-related files. Leave unrelated local work untouched.

Use a Conventional Commit, for example:

```sh
git commit -m "chore(release): bump PV app to 0.1.4"
git push origin main
```

Use `fix` or `feat` instead when the release commit includes the actual bug fix or feature being released.

## Build App Artifacts

After confirming inputs with the user, dispatch the app release workflow:

```sh
gh workflow run app-release.yml \
  --ref <ref> \
  -f minimum_pv_version=<minimum-pv-version> \
  -f app_platforms=<platforms>
```

Watch the run:

```sh
gh run watch <app-release-run-id> --exit-status --interval 30
```

If a job fails, inspect logs before rerunning:

```sh
gh run view <app-release-run-id> --job <job-id> --log
```

Rerun failed jobs only when the evidence shows an environmental failure:

```sh
gh run rerun <app-release-run-id> --failed
```

After success, confirm handoff artifacts exist:

```sh
gh api repos/<owner>/<repo>/actions/runs/<app-release-run-id>/artifacts \
  --jq '.artifacts[] | {name, size_in_bytes, expired}'
```

Expected handoff contents include:

```txt
pv/<version>/pv-<platform>
pv/records/<version>/pv-<platform>.json
pv-app-manifest.json
install.sh
```

## Publish App Artifacts

Publication uploads immutable app binaries and versioned app metadata, then updates stable `install.sh` and `pv-app-manifest.json`.

Confirm `source_run_id` and git ref with the user, then dispatch from the same commit used by the successful app release run:

```sh
gh workflow run app-publication.yml \
  --ref <ref> \
  -f source_run_id=<app-release-run-id>
```

Watch:

```sh
gh run watch <app-publication-run-id> --exit-status --interval 15
```

Inspect the uploaded publication plan when useful:

```sh
gh run download <app-publication-run-id> \
  --name pv-app-publication-plan-<source-run-id>-<publication-run-id> \
  --dir /tmp/pv-app-publication-plan
jq . /tmp/pv-app-publication-plan/publication-plan.json
```

## Verify Stable App Release

After publication succeeds, verify both stable entrypoints:

```sh
curl -fsSL <r2-public-base-url>/pv-app-manifest.json | jq .
curl -fsSL <r2-public-base-url>/install.sh | sed -n '1,80p'
```

Check the manifest version, assets, checksums, and provenance:

```sh
curl -fsSL <r2-public-base-url>/pv-app-manifest.json | jq -r '
  .version as $version
  | .minimum_pv_version as $minimum
  | .assets[]
  | [$version, $minimum, .platform, .url, .sha256, (.size|tostring)] | @tsv
'
```

The published object layout should include:

```txt
pv/<version>/pv-darwin-arm64
pv/records/<version>/pv-darwin-arm64.json
pv/manifests/runs/<source-run-id>/pv-app-manifest.json
pv/manifests/runs/<source-run-id>/install.sh
pv-app-manifest.json
install.sh
```

Run an update check against the published release if a test machine is available:

```sh
pv update --check
```

## Failure Meanings

- `source PV App Release run must use this commit`: build and publication were dispatched from different commits. Rebuild on the publication ref or publish from the source run commit.
- `candidate app version must not be older than current stable`: the stable app manifest or installer already advertises a newer version. Confirm the intended version before proceeding.
- missing `pv-app-manifest.json` or `install.sh`: the app release handoff artifact is incomplete; inspect the release run before publishing.
- missing required `pv-darwin-arm64`: current publication requires the Apple Silicon app binary. Rebuild with `app_platforms` including `darwin-arm64`.
- immutable app object already exists with different content: do not overwrite. Confirm whether this is a version/object-key collision or an unintended rebuild with changed bytes.
- checksum or size mismatch: the binary does not match its release record. Treat the source run as invalid until the cause is understood.

## Final Report

Report:

- version released
- commit SHA and ref
- local verification commands and outcomes
- `PV App Release` run URL and conclusion
- app handoff artifacts confirmed
- `PV App Publication` run URL and conclusion, if published
- manifest and installer verification result
- unrelated local changes left untouched
