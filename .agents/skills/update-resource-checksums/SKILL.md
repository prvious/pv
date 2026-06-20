---
name: update-resource-checksums
description: Verify and update PV Managed Resource source checksums. Use when a resource recipe source_sha256, php_source_sha256, source_url, upstream archive checksum, checksum mismatch, or source provenance needs investigation without necessarily moving to a new upstream version.
---

# Update Resource Checksums

Requested operation: $ARGUMENTS

Use this workflow to investigate and update source checksums for PV Managed Resource recipes. Do not blindly replace a checksum: first decide whether the source identity changed, the upstream asset was reissued, or the existing checksum was simply wrong.

## Choose The Correct Path

- New upstream version: use the upstream-version release workflow instead.
- Same upstream version, artifact already published, source bytes changed: bump `pv_build_revision` before rebuilding because the artifact contents changed for the same upstream identity.
- Same upstream version, artifact not published yet, checksum was wrong: update only the checksum and rebuild the unpublished candidate.
- Build log says source checksum mismatch: verify the URL and bytes independently before editing.

## Before Editing

1. Read repo rules and artifact design:

   ```sh
   sed -n '1,180p' CONTRIBUTING.md
   sed -n '819,917p' DESIGN.md
   git status --short --branch
   ```

2. Inspect current recipe and workflow inputs:

   ```sh
   sed -n '1,180p' .github/workflows/artifact-recipes.yml
   sed -n '1,260p' .github/workflows/artifact-publication.yml
   rg -n 'source_url|source_sha256|php_source_url|php_source_sha256|pv_build_revision|upstream_version' release/artifacts/recipes
   ```

3. Confirm with the user before dispatching any GitHub workflow:
   - resource
   - track
   - platform
   - git ref
   - whether the artifact identity is already published
   - whether to bump `pv_build_revision`
   - whether to publish after a successful build

## Locate The Source Fields

Common recipe files:

```txt
release/artifacts/recipes/php/tracks.toml
release/artifacts/recipes/composer/composer.toml
release/artifacts/recipes/<resource>/recipe.toml
```

Common checksum shapes:

```toml
source_url = "https://..."
source_sha256 = "..."
```

```toml
php_source_url = "https://..."
php_source_sha256 = "..."
```

```toml
[[tracks.sources]]
platform = "darwin-arm64"
source_url = "https://..."
source_sha256 = "..."
```

For PHP, verify both the per-track PHP source and the shared `[frankenphp]` source when relevant. For platform-specific binary recipes such as RustFS or Mailpit, verify each platform source independently.

## Verify The Upstream Bytes

Download to a temporary location and compute SHA-256:

```sh
tmpdir=$(mktemp -d)
curl -L --fail --show-error --silent \
  --retry 3 --retry-delay 2 --retry-all-errors \
  '<source-url>' \
  -o "$tmpdir/source"
shasum -a 256 "$tmpdir/source"
```

Compare against:

- the recipe checksum
- the checksum reported in the failed build log, if any
- the checksum in the published manifest provenance, when the artifact is already public
- upstream release notes or checksums, when upstream publishes them

If the upstream asset changed for the same version and a matching artifact identity is already public, do not keep the old `pv_build_revision`. Publish a new PV revision such as `pv2`.

## Edit The Recipe

Change the smallest relevant fields:

- update `source_sha256` or `php_source_sha256` when the URL is still correct
- update `source_url` only when the source location moved
- keep `upstream_version` unchanged for same-version checksum repair
- bump `pv_build_revision` only when the rebuilt artifact should become a new PV revision

Do not update unrelated tracks, platforms, or resource recipes.

## Local Verification

Focused checks:

```sh
cargo nextest run -p pv-release --test recipe_metadata
cargo nextest run -p pv-release --test recipe_fixtures
cargo nextest run -p pv-release --test release_records
```

If snapshots change, inspect them first and accept only expected checksum/provenance changes:

```sh
cargo insta accept --all
```

Final local verification:

```sh
cargo fmt --all -- --check
cargo insta pending-snapshots --workspace
git diff --check
cargo nextest run -p pv-release
cargo clippy -p pv-release --all-targets --all-features --locked -- -D warnings
```

## Build And Publish

After confirming exact inputs with the user, dispatch the existing artifact build workflow:

```sh
gh workflow run artifact-recipes.yml \
  --ref <ref> \
  -f resource=<resource> \
  -f track=<track> \
  -f platform=<platform>
```

Watch and inspect logs:

```sh
gh run watch <recipes-run-id> --exit-status --interval 30
gh run view <recipes-run-id> --job <job-id> --log
```

If publishing, confirm publication inputs and use the artifact publication workflow:

```sh
gh workflow run artifact-publication.yml \
  --ref <ref> \
  -f source_run_id=<recipes-run-id> \
  -f versioned_manifest_prefix=manifests/runs \
  -f required_native_platforms=<platforms>
```

Verify the stable manifest provenance after publication:

```sh
curl -fsSL <artifact-manifest-url> | jq -r '
  .resources[]
  | select(.name == "<resource>")
  | .tracks[]
  | select(.name == "<track>")
  | .artifacts[]
  | select(.platform == "<platform>")
  | [.artifact_version, .provenance.source_url, .provenance.source_sha256, .provenance.build_run_id]
  | @tsv
'
```

## Failure Meanings

- source checksum mismatch: the downloaded source bytes do not match recipe metadata. Verify the URL and upstream source before editing.
- duplicate artifact identity: the rebuilt candidate matches an already-published resource, track, upstream version, PV revision, and platform. Bump `pv_build_revision` if the artifact contents changed.
- source Artifact Recipes run must use this commit: build and publication were dispatched from different commits.
- checksum changed for a same-version upstream asset: treat as a release decision, not a mechanical checksum replacement.

## Final Report

Report:

- resource, track, and platform
- old and new checksum values
- whether `source_url`, `upstream_version`, or `pv_build_revision` changed
- how the source bytes were verified
- local verification commands and outcomes
- build run URL and conclusion, if dispatched
- publication run URL and manifest verification, if published
- unrelated local changes left untouched
