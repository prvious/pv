# PV Managed Resource Artifact Releases

This tree is the local-first input and output area for PV Managed Resource artifact release tooling.

PR 23 models release records, revocation records, local archive validation, and manifest generation on disk first. Object storage upload, GitHub Actions publication, and stable remote manifest pointer updates are PR 25 work.

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

Both recipe TOML files use a shared `[recipe]` plus `[[tracks]]` schema. Resource-specific sections are only used when the resource family needs extra build metadata.

`recipes/php/tracks.toml` is the data source for paired PHP and FrankenPHP artifact builds. Each selected PHP track/platform is built once with StaticPHP v3, producing both the standalone `php` binary and the matched `frankenphp` binary from the same buildroot. The recipe pins PHP tracks, source URLs, checksums, the expected extension set, the macOS deployment target, and the FrankenPHP source version used by the pair.

`recipes/composer/composer.toml` is the data source for Composer track `2`. Composer is packaged as a `platform: "any"` artifact.

`default-tracks.toml` gives the manifest generator explicit default tracks for generated resources, including Composer's single generated track.

## Local Validation

Run the cheap recipe checks from the repository root:

```shell
shellcheck release/artifacts/recipes/common.sh release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh
```

```shell
rm -rf /tmp/pv-recipe-fixtures
cargo run -p pv-release -- generate-recipe-fixtures \
  --php release/artifacts/recipes/php/tracks.toml \
  --composer release/artifacts/recipes/composer/composer.toml \
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

These local checks validate recipe shell syntax, recipe metadata, generated fixture records, and manifest generation. They do not build real PHP or FrankenPHP artifacts. Real PHP and FrankenPHP artifacts are built only by the manual `Artifact Recipes` GitHub Actions workflow on native macOS runners.

The manual `Artifact Recipes` workflow treats `resource=php` as a PHP-family build: each selected PHP track/platform produces both `php` and `frankenphp` artifacts. Composer remains independently selectable as `resource=composer`.

## Cloudflare R2 Publication

`Artifact Publication` is a manual workflow. It publishes outputs from a prior `Artifact Recipes` workflow run by run ID.

Required configuration:

- Secret `CLOUDFLARE_ACCOUNT_ID`
- Secret `R2_ACCESS_KEY_ID`
- Secret `R2_SECRET_ACCESS_KEY`
- Variable `R2_BUCKET`
- Variable `R2_PUBLIC_BASE_URL`

Publication downloads the selected workflow run artifacts, downloads existing release records and revocations from R2 when present, validates archives and release records again, stages immutable archive and record uploads, writes a versioned manifest under `manifests/runs/$SOURCE_RUN_ID/manifest.json`, and overwrites stable `manifest.json` last. Failed validation or immutable object collision stops before the stable manifest update.
