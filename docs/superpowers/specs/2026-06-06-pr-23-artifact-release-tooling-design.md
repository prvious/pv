# PR 23 Artifact Release Tooling Design

## Context

PR 23 implements `PV-108`, `PV-109`, and `PV-110` from `IMPLEMENTATION.md`: artifact release metadata, manifest generation tooling, and the common artifact packaging and validation harness.

The design source of truth is `DESIGN.md`. Managed Resource artifact releases are separate from PV application releases. PV clients consume a remote Managed Resource artifact manifest, but the public upload and object-storage publication workflow is later PR 25 work. PR 23 should prove the artifact release contract locally first.

## Decision

PR 23 will build a local-first artifact release pipeline.

The pipeline input is structured release metadata and append-only revocation metadata stored as local files. A Rust release tool validates those records, validates local artifact archives, merges release plus revocation state, and emits the same Managed Resource artifact manifest schema already consumed by `crates/resources`.

The local pipeline is the authoritative model for the later cloud workflow, but PR 23 will not implement R2/S3 upload, GitHub Actions publication, CDN invalidation, or stable remote pointer mutation.

## Boundaries

- `crates/resources` remains the client-side manifest parser, selector, and installer-facing contract.
- New internal release tooling owns release records, revocation records, manifest generation, and publication-input validation.
- A `release/artifacts/`-style tree owns local schemas, fixtures, and harness scripts.
- Actual resource recipes are mostly deferred to PR 24 and PR 25. PR 23 may include tiny fixtures or demo hooks only when needed to prove the shared harness.

## Local Data Flow

1. A candidate artifact archive exists locally, such as a `.tar.gz` built by a future recipe.
2. The release tool validates the archive:
   - exactly one top-level directory
   - safe paths only
   - expected metadata fields present
   - license and notice files present where the record says they are required
   - optional smoke-test hooks can be declared and executed locally
3. The tool computes checksum and size from the actual archive.
4. The tool writes or validates an immutable release record containing resource, track, upstream version, PV build revision, artifact version, platform, object key or future URL path, checksum, size, provenance, and `published_at`.
5. Separate revocation records can be added without mutating release records.
6. The manifest generator reads release records plus revocation records and writes manifest JSON that round-trips through `resources::ArtifactManifest::parse`.

Because upload is deferred, PR 23 can use deterministic placeholder/public URL construction from an object-key base such as `https://artifacts.example.test/...` for generated manifests.

## Error Handling

Release metadata, manifest generation, archive validation, smoke hook execution, and revocation merging should expose typed domain errors. The release tool may convert those errors to `anyhow` at the CLI/tool boundary.

These failures are hard stops:

- invalid release record shape
- duplicate artifact identity
- release record checksum or size not matching the archive
- revocation pointing at a missing artifact
- duplicate or contradictory revocation records
- generated manifest rejected by `ArtifactManifest::parse`
- archive missing one top-level root
- unsafe archive paths or special entries
- missing license or notice metadata required by the record
- smoke-test hook failure

PR 23 validates enough to prevent invalid local records and manifests, but it does not define resource-specific lifecycle rules. Adapter-required files such as `bin/redis-server` still belong to resource adapters and later recipes unless the harness is running a declared smoke or file-presence check.

## Testing

PR 23 should be fixture-driven and local-only. Coverage should include:

- release record parsing and validation
- immutable release record identity rules
- append-only revocation merge behavior
- generated manifest shape using `insta` snapshots
- generated manifest round-tripping through `resources::ArtifactManifest::parse`
- checksum and size computed from real fixture archives
- archive validation rejecting multi-root, rootless, path escape, symlink, hardlink, and special-node cases
- smoke hook success and failure handling using tiny local scripts
- publication-input validation that proves versioned and stable manifest files are locally generated correctly, while real upload remains out of scope

Verification should favor focused `cargo nextest` runs for the new release-tool crate/tests plus any existing `resources` tests touched. Snapshot assertions are preferred for generated manifest and error summaries.
