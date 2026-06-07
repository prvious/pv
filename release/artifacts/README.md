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

`recipes/php/tracks.toml` is the data source for PHP and FrankenPHP artifact builds. It pins PHP tracks, source URLs, checksums, the expected extension set, the macOS deployment target, and the FrankenPHP source version used by the recipe.

`recipes/composer/composer.toml` is the data source for Composer track `2`. Composer is packaged as a `platform: "any"` artifact.

`default-tracks.toml` gives the manifest generator explicit default tracks for generated resources, including Composer's single generated track.
