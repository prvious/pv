# PR 25 Artifact Publication and Backing Resource Recipes Design

## Context

PR 25 implements `PV-111`, `PV-114`, `PV-115`, `PV-116`, and `PV-117` from `IMPLEMENTATION.md`.

The design source of truth is `DESIGN.md`. Managed Resource artifacts are published separately from PV application releases. PV clients consume a stable Managed Resource artifact manifest URL that returns full manifest JSON. The manifest is generated from immutable release records plus append-only revocation records. Native macOS artifacts are built and validated on native macOS runners for `darwin-arm64` and `darwin-amd64`; Docker is not an accepted publication path.

PR 23 built the local-first release record, revocation, archive validation, and manifest-generation tooling. PR 24 added PHP/FrankenPHP and Composer artifact recipes plus the dispatch-only native build workflow. PR 25 extends those foundations to object-storage publication and backing Managed Resource artifact recipes.

Cloudflare R2 is the concrete object storage target. The workflow uses R2's S3-compatible API endpoint, configured through GitHub secrets and variables. The repository does not hardcode account IDs, bucket names, credentials, or public hostnames.

## Decision

PR 25 will be delivered as a stacked set of PRs under one roadmap umbrella:

```text
origin/main
  └─ feat/pr25-publication-foundation
       ├─ feat/pr25-redis-recipe
       ├─ feat/pr25-sql-recipes
       ├─ feat/pr25-mailpit-rustfs-recipes
       └─ feat/pr25-publish-matrix
```

`feat/pr25-publication-foundation` owns the shared release contracts: R2 publication workflow, shared recipe helpers, fixture generation, default-track expansion, documentation, and cheap validation.

The resource branches are based on the foundation branch and can run in parallel:

- `feat/pr25-redis-recipe` adds the Redis recipe and smoke validation.
- `feat/pr25-sql-recipes` adds MySQL and Postgres recipes and smoke validation.
- `feat/pr25-mailpit-rustfs-recipes` adds Mailpit and RustFS recipes and smoke validation.
- `feat/pr25-publish-matrix` converges the resource branches and proves the full published matrix.

After implementation-plan generation, the final plan will be converted into Solo task items with blocker relationships that match this branch graph. Foundation work blocks every resource lane. Resource lanes block final matrix publication.

## Publication Data Flow

The existing `Artifact Recipes` workflow remains a manual native-build workflow. It builds selected artifacts, validates them, generates archives, release records, and a local manifest, then uploads those outputs as GitHub Actions artifacts only.

PR 25 adds a separate manual `Artifact Publication` workflow. It accepts a source workflow run ID, downloads that run's artifact bundle, validates the archives and release records again, merges them with existing published R2 release and revocation records, generates a complete manifest, and validates that manifest through `resources::ArtifactManifest::parse`.

Publication writes to R2 in this order:

1. Upload immutable artifact archives under `resources/<resource>/<track>/<artifact-version>/<platform>/...tar.gz`.
2. Upload immutable release records under matching `records/...json` keys.
3. Generate and upload an immutable versioned manifest copy under a run- or timestamp-qualified `manifests/.../manifest.json` key.
4. Overwrite the stable full manifest object, such as `manifest.json`, last.

The stable manifest remains a direct full JSON manifest. PR 25 does not introduce an index or pointer file, because that would require client manifest-fetch behavior changes.

The workflow configuration is supplied by GitHub secrets and variables:

- `CLOUDFLARE_ACCOUNT_ID`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_BUCKET`
- `R2_PUBLIC_BASE_URL`

`R2_PUBLIC_BASE_URL` is the HTTPS base URL used by generated manifests for artifact download URLs. The upload endpoint is derived from the Cloudflare account ID and uses the R2 S3-compatible endpoint with region `auto`.

## Recipe Model

Backing Managed Resources get separate recipe directories:

```text
release/artifacts/recipes/
  redis/
  mysql/
  postgres/
  mailpit/
  rustfs/
```

Each recipe has its own TOML metadata and build/smoke scripts, but they use shared helpers where behavior is common.

Shared Rust and shell helpers own:

- parsing the common `[recipe]` and `[[tracks]]` metadata shape
- validating resource names, tracks, platforms, checksums, `minimum_pv_version`, license/notice metadata, and `pv_build_revision`
- deriving archive root, object key, record key, and artifact version consistently
- generating fixture archives and release records for cheap CI
- writing shell-safe recipe environment output
- packaging normalized single-root `.tar.gz` archives
- writing release records from the actual archive checksum and size
- generating and validating manifests from records

Resource-specific scripts own only the behavior that differs by resource:

- Redis builds or wraps Redis, packages `redis-server` and `redis-cli`, then smoke-tests `PING`/`PONG`.
- MySQL prefers official upstream binaries when suitable, otherwise builds from source, packages server and client tools, then smoke-tests init, start, admin connection, `SELECT 1`, and clean stop.
- Postgres prefers official upstream binaries when suitable, otherwise builds from source, packages server and client tools, then smoke-tests `initdb`, start, `psql SELECT 1`, and clean stop.
- Mailpit wraps upstream static macOS release binaries, then smoke-tests HTTP UI readiness and SMTP port binding.
- RustFS wraps upstream release binaries when suitable, then smoke-tests S3 API readiness and test bucket create/list behavior.

The shared helper layer is not a generic script that builds every resource. It standardizes release metadata, packaging, records, fixtures, and manifest compatibility. Each resource keeps control of native source acquisition, build steps, and runtime smoke validation.

## Resource Matrix

The target matrix is complete:

```text
redis    darwin-arm64, darwin-amd64
mysql    darwin-arm64, darwin-amd64
postgres darwin-arm64, darwin-amd64
mailpit  darwin-arm64, darwin-amd64
rustfs   darwin-arm64, darwin-amd64
```

The design does not intentionally scope down to a preview matrix. If a resource/platform cannot pass native validation during implementation, that lane should be marked blocked with evidence instead of silently dropping it from the target matrix.

Default-track metadata must include every generated resource. The final published manifest includes PHP, FrankenPHP, Composer, and all backing Managed Resource artifacts needed by public setup and update flows.

## Error Handling and Safety

Publication is fail-closed. If validation fails, the workflow stops before updating stable `manifest.json`.

Hard failures include:

- missing or malformed downloaded GitHub artifact bundle
- archive checksum or size mismatch against release records
- duplicate artifact identity in candidate or existing records
- invalid object key, record key, provenance, or source checksum
- missing license or notice files
- failed resource-specific smoke test in the recipe workflow
- generated manifest rejected by `resources::ArtifactManifest::parse`
- missing default-track metadata for any multi-track resource
- R2 upload failure for any archive, record, or versioned manifest

The stable manifest is updated only after all immutable uploads and manifest validation succeed. If the stable update fails, already uploaded archives and records remain unpublished candidates because PV clients only see artifacts referenced by stable `manifest.json`.

The normal path refuses to overwrite immutable archive and release-record keys. Stable `manifest.json` is the only intentionally overwritten R2 object. Repair behavior for immutable keys is out of scope unless it is deliberately designed later.

## Testing

Default local and CI checks stay cheap. They should cover:

- committed recipe metadata parsing for PHP, Composer, Redis, MySQL, Postgres, Mailpit, and RustFS
- default-track coverage for every generated resource
- fixture archive and release-record generation for the full matrix
- generated manifest snapshots and `resources::ArtifactManifest::parse` round trips
- shell syntax or lint checks for all recipe scripts
- publication workflow validation logic using local fixtures or non-stable staged R2 keys, without mutating real stable objects

Manual native workflow checks cover real artifacts:

- Redis `redis-server` plus `redis-cli ping`
- MySQL init, start, admin `SELECT 1`, and stop
- Postgres `initdb`, start, `psql SELECT 1`, and stop
- Mailpit HTTP UI and SMTP bind
- RustFS S3 readiness and test bucket create/list
- archive validation and release-record generation after smoke tests

Final release-candidate verification runs the recipe workflow for the full matrix, then runs the separate publication workflow by source run ID against R2, confirms the versioned manifest and stable `manifest.json` are present, and confirms the stable manifest includes PHP, FrankenPHP, Composer, and all backing-resource artifacts.

## Boundaries

PR 25 does not:

- change the public artifact manifest schema
- change client manifest-fetch behavior
- implement backing Managed Resource adapters
- change Project config syntax or env placeholder behavior
- publish from normal PV application CI
- publish artifacts that have not passed resource-specific smoke validation
- use Docker to build or validate published macOS artifacts
- hardcode Cloudflare account IDs, bucket names, credentials, or public hostnames

## Verification

The implementation plan should prefer focused checks:

- `cargo nextest run -p pv-release --locked`
- `cargo insta test --accept --test-runner nextest -p pv-release -- <changed_test_name>` for intended snapshot updates
- relevant `resources` manifest tests if generated manifest behavior changes
- recipe fixture generation and manifest generation in a fresh temporary directory
- `shellcheck` for all recipe scripts when available
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`

The final implementation plan must also create Solo task items with blockers for foundation work, each resource lane, and final matrix publication.
