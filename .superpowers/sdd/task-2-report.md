# Task 2 Report: PHP Extension Metadata And Overlay Helpers

## Status

DONE_WITH_CONCERNS

## Summary

- Added `resources::php_extensions` with artifact metadata parsing, request resolution, runtime overlay writing, and runtime environment helpers.
- Exported `PhpExtensionModule`, `PhpExtensionLoadKind`, `PhpExtensionResolution`, `PHP_EXTENSION_METADATA_PATH`, and the new helper functions from `resources`.
- Added optional `php_extensions` artifact metadata parsing to `ManifestArtifact` with a public `php_extensions()` accessor.
- Added integration coverage for metadata resolution and runtime overlay generation.
- Updated manifest snapshots to include parsed PHP extension metadata and default empty metadata for older manifests.

## TDD Notes

- Wrote `crates/resources/tests/php_extensions.rs` and extended `manifest_foundation.rs` before implementation.
- Red check failed as expected with missing exports and missing `ManifestArtifact::php_extensions()`.
- Implemented the minimal resources-side helpers and manifest parsing needed to satisfy the tests.

## Verification

- PASS: `cargo nextest run -p resources -E 'test(resolves_available_and_ignored_php_extensions_from_artifact_metadata) or test(writes_runtime_overlay_for_loaded_php_extensions) or test(manifest_parses_registry_backed_resources_tracks_and_artifacts)'`
- PASS: `cargo nextest run -p resources --no-fail-fast`
- PASS: `cargo clippy -p resources --all-targets --all-features --locked -- -D warnings`
- EXPECTED FAIL: `cargo build --workspace --all-targets` fails only in known staged daemon callers using `Option<PhpConfig>::as_deref()` at `crates/daemon/src/project_env.rs:119` and `crates/daemon/src/gateway.rs:482`.

## Scope Notes

- `crates/resources/src/runtime.rs` was listed in the task file, but no runtime adapter change was needed. Requiring PHP extension metadata during artifact validation would violate the approved design requirement that older PHP artifacts without metadata remain valid and are treated as supporting no optional extensions.

## Concerns

- Workspace build remains blocked by the known Task 1 downstream `PhpConfig` API migration work outside this task's scope.
