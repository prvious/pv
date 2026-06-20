---
name: release-resource-artifacts
description: Build, update, revise, and publish PV Managed Resource artifacts through GitHub Actions. Use when the user asks to build resource artifacts, release a new resource revision, bump pv_build_revision, publish artifacts, rerun artifact recipes, inspect artifact workflow inputs, or update the public/staging artifact manifest for PV resources such as php, frankenphp, composer, redis, mysql, postgres, mailpit, or rustfs.
---

# Release Resource Artifacts

Requested operation: $ARGUMENTS

Use this workflow for PV Managed Resource artifact releases. Treat GitHub Actions dispatches as release operations: inspect inputs first, confirm them with the user, then run and verify.

## Before Dispatch

1. Read repository instructions that apply to the workspace, especially `CONTRIBUTING.md` and `DESIGN.md`.
2. Inspect current git state:

   ```sh
   git status --short --branch
   git log --oneline -5
   ```

3. Inspect available workflow inputs before choosing values:

   ```sh
   sed -n '1,180p' .github/workflows/artifact-recipes.yml
   sed -n '1,260p' .github/workflows/artifact-publication.yml
   ```

   Prefer the checked-in workflow on the ref that will be dispatched. If local files may be stale, also inspect the remote workflow with `gh workflow view`.

4. Confirm exact workflow inputs with the user before dispatching any build or publication workflow. Include:
   - workflow file
   - git ref
   - resource
   - track
   - platform
   - source run ID, for publication
   - required native platforms, for publication

Do not silently rely on workflow defaults. Do not reuse old run IDs or old inputs without re-confirming them.

## Choose Operation

- **Build only**: User wants candidate artifacts but not public manifest publication.
- **Publish existing build**: User already has a successful `Artifact Recipes` run ID.
- **New build revision**: Artifact contents, packaging, patches, validation, or build flags changed for the same upstream version.
- **Full release**: Merge or push the release commit, build on the publish ref, then publish.

For public/stable publication, the publication workflow validates that the source recipe run used the same commit as the publication run. Build and publish from the same ref and commit, normally `main`.

## New Build Revision

If artifact contents changed while upstream versions stayed the same, bump the recipe's `pv_build_revision`. A new GitHub run ID is provenance, not an artifact revision.

Common recipe locations:

```txt
release/artifacts/recipes/php/tracks.toml
release/artifacts/recipes/composer/composer.toml
release/artifacts/recipes/<resource>/recipe.toml
```

Change only the relevant resource from, for example:

```toml
pv_build_revision = "pv1"
```

to:

```toml
pv_build_revision = "pv2"
```

Update nearby tests and snapshots that encode artifact versions, object keys, release records, archive roots, or manifests. Prefer existing `pv-release` test patterns.

Useful verification for recipe metadata changes:

```sh
cargo nextest run -p pv-release --test recipe_metadata
cargo nextest run -p pv-release --test recipe_fixtures
cargo fmt --all -- --check
cargo insta pending-snapshots --workspace
git diff --check
cargo nextest run -p pv-release
cargo clippy -p pv-release --all-targets --all-features --locked -- -D warnings
```

Commit with a Conventional Commit message, for example:

```sh
git commit -m "fix(release): bump PHP artifact revision"
```

## Build Artifacts

After confirming inputs with the user, dispatch:

```sh
gh workflow run artifact-recipes.yml \
  --ref <ref> \
  -f resource=<resource> \
  -f track=<track> \
  -f platform=<platform>
```

Watch the run:

```sh
gh run watch <recipes-run-id> --exit-status --interval 30
```

If validation fails due a transient registry/network error, inspect logs before rerunning:

```sh
gh run view <recipes-run-id> --job <job-id> --log
```

Rerun failed jobs only after confirming the failure is environmental:

```sh
gh run rerun <recipes-run-id> --failed
```

After success, confirm artifacts exist:

```sh
gh api repos/<owner>/<repo>/actions/runs/<recipes-run-id>/artifacts \
  --jq '.artifacts[] | {name, size_in_bytes, expired}'
```

## Publish Artifacts

Confirm publication inputs with the user before dispatch:

- `source_run_id`
- `versioned_manifest_prefix`
- `required_native_platforms`
- git ref

Dispatch:

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

After success, verify the manifest points at the intended build revision and run ID:

```sh
curl -fsSL <artifact-manifest-url> | jq -r '
  .resources[]
  | select(.name == "<resource>")
  | .name as $resource
  | .tracks[]
  | .name as $track
  | .artifacts[]
  | select(.platform == "<platform>")
  | [$resource, $track, .artifact_version, .pv_build_revision, .provenance.build_run_id] | @tsv
'
```

For PHP releases, check both `php` and `frankenphp`.

## Failure Meanings

- `duplicate artifact identity`: the source run produced an identity that is already published. If contents changed for the same upstream version, bump `pv_build_revision`, rebuild, and publish the new run.
- `source Artifact Recipes run must use this commit`: build and publication were dispatched from different commits. Rebuild on the publication ref or publish from the source run's commit.
- missing required native platform: either build the missing platform or confirm an explicit preview gate such as `required_native_platforms=darwin-arm64`.
- immutable object already exists: do not overwrite. Inspect whether this is a retry with matching records or a real identity/object-key collision.

## Final Report

Report:

- commit SHA pushed, if any
- local verification commands and outcomes
- Artifact Recipes run URL and conclusion
- artifact bundle names or count
- Artifact Publication run URL and conclusion, if published
- manifest verification result
- unrelated local changes left untouched
