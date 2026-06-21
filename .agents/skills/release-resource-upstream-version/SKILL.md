---
name: release-resource-upstream-version
description: Update and publish a PV Managed Resource artifact for a new upstream version. Use when the user asks to move a resource track to a newer upstream release, update source_url/source_sha256/upstream_version, build artifacts for a new upstream resource version, or publish a new artifact such as rustfs 1.1.0-beta-pv1, mysql 8.4.x-pv1, postgres 18.x-pv1, redis 8.8.x-pv1, composer 2.x-pv1, mailpit 1.x-pv1, or PHP patch versions.
---

# Release Resource Upstream Version

Requested operation: $ARGUMENTS

Use this workflow when the upstream resource version changes. For the same upstream version with changed PV packaging, use the revision-bump workflow instead.

## Core Rule

- New upstream release: change `upstream_version` and source metadata; use `pv_build_revision = "pv1"` for that upstream version.
- Same upstream release, new PV packaging/build flags/validation: keep `upstream_version`; bump `pv_build_revision` to `pv2`, `pv3`, etc.

Example:

```txt
rustfs 1.0.0-beta.7-pv1 -> rustfs 1.1.0-beta-pv1
```

is a new upstream version, not a new PV revision.

## Before Editing Or Dispatching

1. Read repository instructions and design:

   ```sh
   sed -n '1,160p' CONTRIBUTING.md
   sed -n '780,930p' DESIGN.md
   git status --short --branch
   ```

2. Inspect available GitHub workflow inputs before choosing values:

   ```sh
   sed -n '1,180p' .github/workflows/artifact-recipes.yml
   sed -n '1,260p' .github/workflows/artifact-publication.yml
   ```

   If local workflows may not match the dispatch ref, inspect the remote workflow with `gh workflow view`.

3. Confirm exact workflow inputs with the user before dispatching any build or publication workflow:
   - workflow file
   - git ref
   - resource
   - track
   - platform
   - whether to publish after build
   - source run ID, for publication
   - required native platforms, for publication

Do not silently rely on defaults or reuse old run IDs without confirmation.

## Update Recipe

Find the resource recipe:

```txt
release/artifacts/recipes/php/tracks.toml
release/artifacts/recipes/composer/composer.toml
release/artifacts/recipes/<resource>/recipe.toml
```

Update the resource track's upstream version and source metadata. For backing resources, the shape is usually:

```toml
[[tracks]]
name = "1"
upstream_version = "1.1.0-beta"

[[tracks.sources]]
platform = "darwin-arm64"
source_url = "https://..."
source_sha256 = "..."

[[tracks.sources]]
platform = "darwin-amd64"
source_url = "https://..."
source_sha256 = "..."
```

Usually keep or reset:

```toml
pv_build_revision = "pv1"
```

For PHP, standalone PHP and FrankenPHP tracks must stay paired on the same PHP patch version. Update PHP source URLs/checksums and any FrankenPHP source metadata only when that upstream also changes.

## Update Tests And Snapshots

Update tests and snapshots that encode the old upstream version:

- recipe metadata expectations
- fixture archive roots
- generated release record or manifest snapshots
- smoke tests when source filenames, version output, or archive shape changed
- docs that list current recipe versions, when present

Prefer existing `pv-release` test patterns and `insta` snapshots.

Focused checks:

```sh
cargo nextest run -p pv-release --test recipe_metadata
cargo nextest run -p pv-release --test recipe_fixtures
```

If snapshots change, inspect them first, then accept only expected changes:

```sh
cargo insta accept --all
```

Final local verification before committing:

```sh
cargo fmt --all -- --check
cargo insta pending-snapshots --workspace
git diff --check
cargo nextest run -p pv-release
cargo clippy -p pv-release --all-targets --all-features --locked -- -D warnings
```

## Commit And Push

Stage only files related to the upstream version update. Leave unrelated local work untouched.

Use a Conventional Commit, for example:

```sh
git commit -m "feat(release): update RustFS to 1.1.0-beta"
git push origin main
```

Use `feat(release)` because a new installable artifact version becomes available.

## Confirm Build Dispatch

Immediately before dispatching `artifact-recipes.yml`, restate the finalized build inputs:

- workflow file
- git ref
- resource
- track
- platform
- whether publication will be considered after the build

Confirm with the user that this exact build dispatch should proceed. A prior input confirmation does not satisfy this gate.

Do not continue to `Build Artifacts` unless the user explicitly confirms this workflow dispatch.

## Build Artifacts

After the build dispatch has been confirmed, dispatch:

```sh
gh workflow run artifact-recipes.yml \
  --ref <ref> \
  -f resource=<resource> \
  -f track=<track> \
  -f platform=<platform>
```

Watch:

```sh
gh run watch <recipes-run-id> --exit-status --interval 30
```

If a job fails, inspect logs before deciding whether to rerun:

```sh
gh run view <recipes-run-id> --job <job-id> --log
```

Rerun only when the evidence shows an environmental failure:

```sh
gh run rerun <recipes-run-id> --failed
```

Confirm uploaded artifacts:

```sh
gh api repos/<owner>/<repo>/actions/runs/<recipes-run-id>/artifacts \
  --jq '.artifacts[] | {name, size_in_bytes, expired}'
```

## Confirm Publication Dispatch

After the build completes and uploaded artifacts are verified, restate the finalized publication inputs:

- workflow file
- git ref
- source run ID
- versioned manifest prefix
- required native platforms

Confirm with the user that this exact publication dispatch should proceed, explicitly noting that published artifacts become available to all clients.

Do not continue to `Publish Artifacts` unless the user explicitly confirms this workflow dispatch. A prior build confirmation does not satisfy this gate.

## Publish Artifacts

Publication must use the same commit as the successful recipe run.

After the publication dispatch has been confirmed, dispatch:

```sh
gh workflow run artifact-publication.yml \
  --ref <ref> \
  -f source_run_id=<recipes-run-id> \
  -f versioned_manifest_prefix=manifests/runs \
  -f required_native_platforms=<platforms>
```

Watch:

```sh
gh run watch <publication-run-id> --exit-status --interval 15
```

Verify the stable manifest includes the new upstream artifact:

```sh
curl -fsSL <artifact-manifest-url> | jq -r '
  .resources[]
  | select(.name == "<resource>")
  | .tracks[]
  | select(.name == "<track>")
  | .artifacts[]
  | [.artifact_version, .upstream_version, .pv_build_revision, .platform, .provenance.build_run_id]
  | @tsv
'
```

Expected for a new upstream release:

```txt
artifact_version: <new-upstream-version>-pv1
upstream_version: <new-upstream-version>
pv_build_revision: pv1
```

For PHP, verify both `php` and `frankenphp` resources.

## Failure Meanings

- `duplicate artifact identity`: the candidate used an already-published upstream version, PV revision, and platform. Confirm whether this is really a new upstream release or should be a PV revision bump.
- `source Artifact Recipes run must use this commit`: build and publication were dispatched from different commits. Rebuild on the publication ref or publish from the source run's commit.
- missing required native platform: build the missing platform or confirm an explicit preview gate such as `required_native_platforms=darwin-arm64`.
- source checksum mismatch: update the source SHA only after independently verifying the upstream asset and URL are correct.

## Final Report

Report:

- old and new upstream versions
- `pv_build_revision` used
- commit SHA pushed, if any
- local verification commands and outcomes
- Artifact Recipes run URL and conclusion
- artifact bundle names or count
- Artifact Publication run URL and conclusion, if published
- manifest verification result
- unrelated local changes left untouched
