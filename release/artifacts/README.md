# PV Managed Resource Artifact Releases

This tree is the local-first input and output area for PV Managed Resource artifact release tooling.

Release records, revocation records, local archive validation, manifest generation, recipe builds, and publication staging all flow through this tree before public artifact manifests are published.

## Directories

- `records/` stores immutable artifact release records.
- `revocations/` stores append-only revocation records.
- `manifests/` stores generated local manifests.

Release records describe artifacts that already exist locally or will be uploaded later. Revocation records never mutate release records; the manifest generator merges both record streams into the client-facing Managed Resource artifact manifest.

## Local Commands

Generate a local manifest:

```shell
cargo run -p pv-release -- generate-manifest \
  --records release/artifacts/records \
  --revocations release/artifacts/revocations \
  --output release/artifacts/manifests/manifest.json \
  --base-url https://artifacts.example.test
```

Validate a local archive against a release record:

```shell
cargo run -p pv-release -- validate-archive \
  --archive path/to/artifact.tar.gz \
  --record release/artifacts/records/path/to/record.json
```

## Recipes

Recipe TOML files use a shared `[recipe]` plus `[[tracks]]` schema. Resource-specific sections are only used when the resource family needs extra build metadata.

`recipes/php/tracks.toml` is the data source for paired PHP and FrankenPHP artifact builds. Each selected PHP track/platform is built once with StaticPHP v3, producing both the standalone `php` binary and the matched `frankenphp` binary from the same buildroot. The recipe pins PHP tracks `8.3`, `8.4`, and `8.5`; source URLs; checksums; the default loaded extension set; the optional bundled extension catalog; the expected runtime extension set; the macOS deployment target; and the FrankenPHP source version used by the pair. Generated release records and manifests include optional PHP extension metadata so PV can load bundled modules through runtime ini overlays.

`recipes/composer/composer.toml` is the data source for Composer track `2`. Composer is packaged as a `platform: "any"` artifact.

The backing resource recipes cover MySQL tracks `8.0`, `8.4`, and `9.7`; Postgres tracks `17` and `18`; Redis track `8.8`; Mailpit track `1`; and RustFS track `1`.

`default-tracks.toml` gives the manifest generator explicit default tracks for generated resources: PHP/FrankenPHP `8.5`, MySQL `8.4`, Postgres `18`, Redis `8.8`, Composer `2`, Mailpit `1`, and RustFS `1`. MySQL `8.0` is compatibility-only.

## Local Validation

Run the cheap recipe checks from the repository root:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/*/*.sh
```

```shell
rm -rf /tmp/pv-recipe-fixtures
cargo run -p pv-release -- generate-recipe-fixtures \
  --php release/artifacts/recipes/php/tracks.toml \
  --composer release/artifacts/recipes/composer/composer.toml \
  --redis release/artifacts/recipes/redis/recipe.toml \
  --mysql release/artifacts/recipes/mysql/recipe.toml \
  --postgres release/artifacts/recipes/postgres/recipe.toml \
  --mailpit release/artifacts/recipes/mailpit/recipe.toml \
  --rustfs release/artifacts/recipes/rustfs/recipe.toml \
  --archives /tmp/pv-recipe-fixtures/archives \
  --records /tmp/pv-recipe-fixtures/records \
  --pv-commit "$(git rev-parse HEAD)" \
  --build-run-id local
cargo run -p pv-release -- generate-manifest \
  --records /tmp/pv-recipe-fixtures/records \
  --revocations release/artifacts/revocations \
  --defaults release/artifacts/default-tracks.toml \
  --output /tmp/pv-recipe-fixtures/manifest.json \
  --base-url https://artifacts.example.test
```

These local checks validate recipe shell syntax, recipe metadata, generated fixture records, and manifest generation. They do not build real managed-resource artifacts. Real artifacts are built only by the manual `Artifact Recipes` GitHub Actions workflow on native macOS runners.

The manual `Artifact Recipes` workflow treats `resource=php` as a PHP-family build: each selected PHP track/platform produces both `php` and `frankenphp` artifacts. `resource=all` with `track=all` builds every configured track for each resource lane in parallel. A resource-specific single track builds only that resource track. `platform=all` currently resolves native resource lanes to `darwin-arm64` for the Apple Silicon/staging RC, while `darwin-amd64` remains explicitly dispatchable for diagnostics and the conditional `darwin-amd64` gate. Composer remains a single `platform=any` artifact so full-resource runs do not duplicate Composer identities.

## Cloudflare R2 Publication

`Artifact Publication` is a manual workflow. It publishes outputs from a prior `Artifact Recipes` workflow run by run ID.

Required configuration:

- Secret `CLOUDFLARE_ACCOUNT_ID`
- Secret `R2_ACCESS_KEY_ID`
- Secret `R2_SECRET_ACCESS_KEY`
- Variable `R2_BUCKET`
- Variable `R2_PUBLIC_BASE_URL`

Publication downloads the selected workflow run artifacts, downloads existing release records and revocations from R2 when present, validates archives and release records again, stages immutable archive and record uploads, writes a versioned manifest under `manifests/runs/$SOURCE_RUN_ID/manifest.json`, and overwrites stable `manifest.json` last. Failed validation or immutable object collision stops before the stable manifest update.

`required_native_platforms` controls which native platforms must be present in the public manifest before publication. It defaults to `darwin-arm64` for the current preview/default Apple Silicon/staging RC validation while Intel FrankenPHP builds remain deferred. Set it to `darwin-arm64,darwin-amd64` when the future/full public v1 native platform matrix is ready to publish.
