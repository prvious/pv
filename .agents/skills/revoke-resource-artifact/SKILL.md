---
name: revoke-resource-artifact
description: Revoke PV Managed Resource artifacts. Use when a published resource artifact must be marked revoked, a revocation_reason or replacement_artifact_version is needed, update/check should warn about a bad artifact, or release metadata needs emergency artifact revocation.
---

# Revoke Resource Artifact

Requested operation: $ARGUMENTS

Use this workflow when a published Managed Resource artifact must be marked revoked. Revocations are append-only metadata records; never mutate or delete the original release record or archive.

## Current Repo Capability

The code supports revocation records and manifest generation:

- revocation records are parsed by `pv-release`
- generated manifests include `revoked`, `revocation_reason`, `revoked_at`, and optional `replacement_artifact_version`
- clients report revoked installed artifacts and can fall back to a non-revoked latest artifact

The current `Artifact Publication` workflow does not upload new candidate revocation records from the repo and does not support revocation-only publication. It only downloads already-published R2 revocation records and merges them into the generated manifest. Do not claim revocation is publishable through `artifact-publication.yml` alone unless the workflow has changed.

## Before Acting

1. Read repo rules and revocation design:

   ```sh
   sed -n '1,180p' CONTRIBUTING.md
   sed -n '909,919p' DESIGN.md
   git status --short --branch
   ```

2. Inspect current implementation and workflows:

   ```sh
   rg -n 'RevocationRecord|revocation|replacement_artifact_version|published_revocations' crates/pv-release crates/resources crates/cli .github release
   sed -n '1,300p' .github/workflows/artifact-publication.yml
   ```

3. Confirm the exact revocation decision with the user:
   - resource
   - track
   - artifact version
   - platform
   - revocation reason
   - replacement artifact version, if any
   - whether this is emergency manual publication or a code/workflow change first

## Identify The Artifact

Read the stable manifest and confirm the exact target:

```sh
curl -fsSL <artifact-manifest-url> | jq -r '
  .resources[]
  | select(.name == "<resource>")
  | .tracks[]
  | select(.name == "<track>")
  | .artifacts[]
  | [.artifact_version, .platform, .published_at, .revoked, (.revocation_reason // ""), .url]
  | @tsv
'
```

The revocation identity is:

```txt
<resource>:<track>:<artifact_version>:<platform>
```

If a replacement is provided, it must exist for the same resource, track, and platform, must not point to the revoked artifact itself, and must not also be revoked.

## Create A Revocation Record

Preferred local path:

```txt
release/artifacts/revocations/resources/<resource>/<track>/<artifact-version>/<platform>/<resource>-<artifact-version>-<platform>.json
```

Record shape:

```json
{
  "resource": "redis",
  "track": "8.8",
  "artifact_version": "8.8.0-pv1",
  "platform": "darwin-arm64",
  "reason": "broken archive",
  "revoked_at": "2026-06-20T12:00:00Z",
  "replacement_artifact_version": "8.8.0-pv2"
}
```

Omit `replacement_artifact_version` only when no non-revoked replacement exists yet. Use an RFC 3339 UTC timestamp ending in `Z`.

## Local Validation

If R2 credentials are available, validate against the published record set:

```sh
published=/tmp/pv-published-revocation-check
rm -rf "$published"
mkdir -p "$published/records" "$published/revocations"
aws s3 sync "s3://$R2_BUCKET/records/" "$published/records" --endpoint-url "$R2_ENDPOINT"
aws s3 sync "s3://$R2_BUCKET/revocations/" "$published/revocations" --endpoint-url "$R2_ENDPOINT"
cp <new-revocation-json> "$published/revocations/<object-path>.json"
cargo run -p pv-release -- generate-manifest \
  --records "$published/records" \
  --revocations "$published/revocations" \
  --defaults release/artifacts/default-tracks.toml \
  --output "$published/manifest.json" \
  --base-url <r2-public-base-url>
```

Then inspect the generated target:

```sh
jq -r '
  .resources[]
  | select(.name == "<resource>")
  | .tracks[]
  | select(.name == "<track>")
  | .artifacts[]
  | select(.artifact_version == "<artifact-version>" and .platform == "<platform>")
  | {artifact_version, platform, revoked, revocation_reason, revoked_at, replacement_artifact_version}
' "$published/manifest.json"
```

Focused test checks:

```sh
cargo nextest run -p pv-release --test release_records
cargo nextest run -p pv-release --test manifest_generation
cargo nextest run -p pv-release --test publication
cargo nextest run -p resources --test manifest_foundation
cargo nextest run -p cli --test update
```

Final local verification for code or checked-in revocation changes, before commit, push, workflow dispatch, publication, or any claim that the revocation change is ready:

```sh
cargo fmt --all --check
cargo insta pending-snapshots --workspace
git diff --check
cargo nextest run --workspace --all-features --locked --no-fail-fast
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Focused revocation checks do not replace the full workspace suite. If the full suite produces expected snapshot drift, inspect the generated snapshots, accept them only through `cargo insta`, then rerun the full workspace suite until it passes.

## Publication Options

Because the current workflow cannot publish a new revocation record by itself, choose one of these paths with the user:

- Add a proper revocation publication workflow or extend `Artifact Publication` to accept candidate revocations.
- If a replacement artifact is also being published, ensure the revocation record is already present in R2 before the artifact publication run generates the stable manifest.
- For emergency manual publication, get explicit user approval before mutating R2 outside the workflow.

For an emergency manual path, confirm all object keys before running commands:

```txt
revocations/resources/<resource>/<track>/<artifact-version>/<platform>/<resource>-<artifact-version>-<platform>.json
manifests/runs/<manual-run-or-ticket>/manifest.json
manifest.json
```

The safe order is gated. Do not batch these mutations under one approval; a prior confirmation does not authorize later R2 mutations.

1. Confirm with the user before uploading the revocation JSON as an immutable object, failing if it already exists. Stop unless the user explicitly approves this upload.
2. Generate and validate a complete manifest from published records plus all revocations.
3. Confirm with the user before uploading the versioned manifest. Stop unless the user explicitly approves this upload.
4. Confirm with the user before updating stable `manifest.json`, explicitly noting that clients will observe the new revocation state after this mutation. Stop unless the user explicitly approves this update.
5. Verify clients see the revoked state.

Do not hand-edit the stable manifest JSON directly.

## Verify Client Behavior

After publication, verify the stable manifest:

```sh
curl -fsSL <artifact-manifest-url> | jq -r '
  .resources[]
  | select(.name == "<resource>")
  | .tracks[]
  | select(.name == "<track>")
  | .artifacts[]
  | select(.artifact_version == "<artifact-version>" and .platform == "<platform>")
'
```

If a machine has the revoked artifact installed, run:

```sh
pv update --check
```

Expected status is `revoked` for the installed artifact, with the reason and replacement when present.

## Failure Meanings

- revocation target missing: no release record exists for the target resource, track, artifact version, and platform.
- replacement release must exist: the replacement artifact version is not published for the same resource, track, and platform.
- replacement must not point at the revoked artifact itself: choose a different replacement or omit the field.
- duplicate or conflicting revocation: a revocation for this identity already exists; do not overwrite it.
- publication workflow has no immutable uploads: current `artifact-publication.yml` is not a revocation-only publication workflow.

## Final Report

Report:

- target artifact identity
- revocation reason and timestamp
- replacement artifact version, if any
- local manifest validation result
- tests run and outcomes
- publication path used or current workflow gap
- manifest verification result
- client `pv update --check` result, when available
- unrelated local changes left untouched
