# PR 24 PHP, FrankenPHP, and Composer Artifact Recipes Design

## Context

PR 24 implements `PV-112` and `PV-113` from `IMPLEMENTATION.md`: artifact recipes for PHP/FrankenPHP and Composer.

The design source of truth is `DESIGN.md`. PHP and FrankenPHP are native macOS runtime artifacts with fixed PHP extensions, exact patch-version sync, macOS 13 deployment target checks, and loopback serving smoke tests. Composer is a portable `platform: "any"` artifact that packages `composer.phar`.

PR 24 is independent from PR 15. Composer recipe validation must not depend on PV's PHP shim or PR 15 command behavior. PR 24 also does not publish artifacts to object storage or mutate stable public manifests; that remains PR 25 scope.

## Decision

PR 24 will add data-driven shell recipes plus lightweight local validation.

PHP and FrankenPHP use one shared recipe driven by committed TOML metadata. The recipe supports the initial PHP tracks `8.2`, `8.3`, and `8.4` from day one. The track matrix is data, not duplicated shell logic.

PHP and FrankenPHP native builds are paired by PHP track. Building PV PHP track `8.4` means building one StaticPHP v3 buildroot for PHP `8.4.x` plus the pinned FrankenPHP source version, then packaging both the standalone `php` binary and the matched `frankenphp` binary from that same buildroot. PV does not support independent PHP-only or FrankenPHP-only native builds in this workflow because the product contract requires CLI and browser execution to use the same PHP patch version and extension set.

Composer uses a smaller committed metadata file for Composer track `2`.

Both metadata files use the same outer TOML schema: a `[recipe]` section for resource-level defaults and `[[tracks]]` entries for selectable tracks. PHP/FrankenPHP adds `[php]` and `[frankenphp]` sections only for native-build metadata that Composer does not need.

Default local and PR validation must not require developers to install PHP, FrankenPHP, Caddy, or heavy native build dependencies. Real PHP/FrankenPHP builds and real Composer PHP smoke tests run only in a manual native macOS CI workflow.

Docker is not used for published macOS artifacts. PV needs native macOS validation for Mach-O linking, signing, rpaths, deployment target, and runtime behavior.

## Recipe Layout

The release tree will gain recipe-owned metadata and scripts:

```text
release/artifacts/recipes/
  php/
    tracks.toml
    build.sh
    smoke.sh
  composer/
    composer.toml
    build.sh
    smoke.sh
```

`release/artifacts/recipes/php/tracks.toml` owns:

- supported track names, initially `8.2`, `8.3`, and `8.4`
- exact PHP patch versions per track
- PHP source URLs and SHA-256 checksums
- shared expected PHP extension list from `DESIGN.md`
- macOS deployment target, currently `13.0`
- any recipe metadata needed to produce release records

`release/artifacts/recipes/composer/composer.toml` owns:

- Composer track `2`
- exact Composer PHAR version
- PHAR source URL and SHA-256 checksum
- license and notice metadata
- release-record provenance defaults that are stable across builds

The scripts read metadata through release-tooling helpers and build a requested resource, track, and platform. Composer remains a single-resource recipe. PHP/FrankenPHP builds are pair-oriented: one selected PHP track/platform produces both normalized single-root `.tar.gz` archives and both structured release records compatible with `pv-release`.

## Paired StaticPHP Build Model

The PHP recipe uses StaticPHP v3 as the native build system. The manual workflow installs or bootstraps a pinned StaticPHP v3 toolchain, then runs one build per selected PHP track/platform.

For a track such as `8.4`, the build inputs are:

- PV track: `8.4`
- PHP source version: for example `8.4.20`
- FrankenPHP source version: for example `1.12.3`
- shared extension set from `tracks.toml`
- macOS deployment target `13.0`

The build command shape is the combined StaticPHP v3 command:

```shell
spc build:php "$PV_BUILD_EXTENSIONS" \
  --build-cli \
  --build-frankenphp \
  --enable-zts \
  --dl-with-php="$PV_PHP_VERSION" \
  --dl-custom-local "php-src:$php_source_dir" \
  --dl-custom-local "frankenphp:$frankenphp_source_dir"
```

The recipe may split the download phase from the build phase if that gives better checksum enforcement or caching, but it must keep the same effective inputs and must still force the verified local PHP and FrankenPHP source trees into the build.

The paired buildroot produces at least:

- `buildroot/bin/php`
- `buildroot/bin/frankenphp`

PV then packages two resources from the same buildroot:

- `php`, track `8.4`, upstream version `8.4.20`
- `frankenphp`, track `8.4`, upstream version `8.4.20-frankenphp1.12.3`

Both release records share the same `pv_commit`, `build_run_id`, recipe path, StaticPHP toolchain provenance, and PHP source input. The FrankenPHP record also records the FrankenPHP source input. If StaticPHP exposes a stable version string, the recipe records it in build logs and, where the release-record schema permits, provenance metadata. If the schema cannot represent toolchain inputs yet, the workflow must at least upload the build log with the exact `spc --version` output.

The standalone PHP binary is expected to be ZTS because FrankenPHP requires a thread-safe PHP build. This is acceptable for PV v1: parity between CLI PHP and the FrankenPHP worker is more important than matching a common NTS CLI distribution. Smoke tests must prove the ZTS CLI handles normal PHP CLI execution and Composer smoke usage before publication.

## Local Data Flow

Local validation is cheap and deterministic:

1. Parse PHP and Composer TOML metadata.
2. Validate required fields, supported track names, version formats, checksums, extension lists, deployment target, and object-key shape.
3. Build tiny fixture archives for tests instead of real PHP/FrankenPHP artifacts.
4. Validate fixture archives and records through `pv-release validate-archive`.
5. Generate a local manifest from release records and round-trip it through `resources::ArtifactManifest::parse`.

This proves the recipe metadata, packaging contract, release records, and manifest compatibility without requiring native runtime builds on developer machines.

## Manual CI Data Flow

PR 24 adds a manual GitHub Actions workflow for real artifact builds.

The workflow uses `workflow_dispatch` and native macOS runners. It accepts selected resource, track, and platform inputs, with an `all` option for the full matrix.

For PHP/FrankenPHP, the workflow:

1. Installs recipe-managed build-time dependencies on the runner, including a pinned or checksum-verified StaticPHP v3 toolchain.
2. Builds standalone PHP and matched FrankenPHP as a pair for each selected PHP track/platform.
3. Enforces the macOS 13 deployment target.
4. Ad-hoc signs binaries when required by the shared harness.
5. Verifies PHP CLI version and expected extensions.
6. Verifies standalone PHP and FrankenPHP use the same PHP patch version.
7. Starts FrankenPHP on loopback with a tiny PHP site, checks the response, and stops it cleanly.
8. Packages separate PHP and FrankenPHP archives and release records from the same buildroot.

Workflow resource selection treats PHP and FrankenPHP as a pair. A full matrix run builds every PHP track/platform pair and Composer. A PHP-family run builds both PHP and FrankenPHP for the selected PHP track/platform. Composer remains independently selectable because it is a portable PHAR artifact rather than a native PHP runtime.

For Composer, the workflow:

1. Downloads the pinned Composer PHAR.
2. Verifies the PHAR checksum.
3. Packages a `platform: "any"` archive containing `composer.phar` and required metadata.
4. Runs real Composer smoke through an explicit CI-provided PHP binary.
5. Packages the archive and release record.

The workflow uploads archives, release records, and generated local manifests as GitHub Actions artifacts only. It does not publish to object storage and does not update stable manifest pointers.

## Error Handling

Recipe scripts should fail fast with clear command names, selected resource, track, platform, and input paths in their messages.

Rust-side metadata parsing and validation should expose typed errors in the release tooling boundary where practical. Tests should snapshot structured error summaries rather than relying on substring assertions.

Hard failures include:

- invalid or missing TOML metadata
- unsupported PHP or Composer track
- checksum mismatch
- unexpected PHP extension set
- PHP/FrankenPHP patch-version mismatch
- PHP/FrankenPHP pair build missing either output binary
- macOS deployment target mismatch
- unmanaged Homebrew runtime paths in native artifacts
- failed loopback serving smoke
- release record rejected by `pv-release`
- generated manifest rejected by `resources`

## Testing

Default verification should cover:

- PHP track metadata parsing and validation
- Composer metadata parsing and validation
- generated release record shape with `insta` snapshots
- fixture archive validation through `pv-release`
- generated manifest snapshots and parser round-trips
- shell script linting with `shellcheck`

Manual CI verification should cover:

- native PHP and FrankenPHP builds on macOS
- paired artifact generation from one StaticPHP buildroot
- exact PHP patch sync between standalone PHP and FrankenPHP
- expected extension set for both CLI and FrankenPHP
- ZTS standalone PHP CLI smoke behavior
- macOS 13 deployment target
- no unmanaged Homebrew runtime paths
- loopback FrankenPHP serving smoke
- Composer PHAR smoke with an explicit PHP binary

## Boundaries

PR 24 does not:

- depend on PR 15 or PV's PHP shim
- publish artifacts to object storage
- mutate stable public manifests
- change the public artifact manifest schema
- require local native PHP/FrankenPHP build dependencies
- use Docker for published macOS artifact validation
- implement PHP, Composer, setup, update, or install command behavior

## Verification

The implementation plan should prefer focused checks:

- `cargo nextest run -p pv-release`
- relevant `resources` manifest tests if release record output changes manifest behavior
- `cargo insta test --accept --test-runner nextest -p pv-release -- <test_name>` only for intended snapshot updates
- `shellcheck release/artifacts/recipes/php/*.sh release/artifacts/recipes/composer/*.sh`
- manual `workflow_dispatch` runs for real native artifacts before release publication work begins
