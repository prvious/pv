# Contributing

## Error Handling

Use both `thiserror` and `anyhow`, with clear boundaries.

Use `thiserror` for domain and library crate errors. Crates such as `state`, `config`, `resources`, `macos`, and daemon internals should expose typed errors so callers can make decisions from variants instead of parsing strings.

Use `anyhow` only at application and orchestration boundaries, such as CLI command handlers, daemon job entrypoints, setup/update orchestration, release tooling, and tests. Add context there before converting errors into user-facing reports.

Domain crates should not expose `anyhow::Result` from public APIs. Prefer `Result<T, CrateError>` where `CrateError` is a `thiserror` enum.

Use typed error variants when PV needs different behavior for different failures, such as invalid Project config, daemon unavailable, protocol mismatch, checksum mismatch, manifest incompatibility, port conflict, non-PV-owned system config, migration failure, or Resource allocation failure.

Tests should prefer asserting typed error variants over substring assertions when the error comes from a domain crate.

Avoid `panic!`, `unreachable!`, `.unwrap()`, and `.expect()` in production code. Propagate or handle fallible behavior explicitly.

## Testing

For running tests, we recommend [nextest](https://nexte.st/).

Daemon tests that exercise Managed Resource, gateway, and supervisor fixtures require `python3` on `PATH`. These fixtures use only Python's standard library; no Python packages or virtual environment is required.

To run a specific test by name:

```shell
cargo nextest run -E 'test(test_name)'
```

To run all tests and accept snapshot changes:

```shell
cargo insta test --accept --test-runner nextest
```

To update snapshots for a specific test:

```shell
cargo insta test --accept --test-runner nextest -- <test_name>
```

## Fixture Lifecycle

Tests that start long-running Managed Resource or Gateway fixtures must register
each intended process's exact PID-path/runtime-metadata-path pair
before startup or reconciliation. Capture assertions while the runtime is live,
then use explicit cleanup where the guard exposes it so cleanup failures fail
the test. If an operation has already failed, preserve that primary failure
while also reporting cleanup failures. Cleanup validates the recorded metadata
and process identity, stops the complete owned process group, waits for its
members and listeners to disappear, and removes the exact records only after
verified stop.

Fixture guards must also clean up from `Drop` so normal return, early return,
panic unwind, and cancellation cannot bypass teardown. The fallback uses an
independent cleanup runtime and reports failures because `Drop` cannot return
them to the test. Long-running fixture entrypoints also monitor their actual
test parent. When that parent disappears, Python fixtures exit themselves and
shell wrappers stop and reap only the child they directly spawned, including
parent-loss races before readiness. The Rust guard or supervisor owns
whole-process-group cleanup.

Use the CI nextest profile for CI runs (`cargo nextest run --profile ci`). It
warns after 60 seconds, terminates a wedged test after 120 seconds, and allows
10 seconds for graceful exit. The default profile remains available for tests
with intentionally longer limits.

Persisted-runtime cleanup must never discover ownership through process-name
scans or signal a PID found only in a record. A persisted PID is actionable
only together with matching recorded metadata and a verified process identity.
A fixture wrapper may signal and reap a child it directly spawned and still
owns. These guarantees do not cover machine or kernel failure, a killed or
stopped parent watcher, or descendants that deliberately escape their owned
process group.

## Formatting

```shell
# Rust
cargo fmt --all
```

## Linting

Linting requires [shellcheck](https://github.com/koalaman/shellcheck) and
[cargo-shear](https://github.com/Boshen/cargo-shear) to be installed separately.

```shell
# Rust
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Shell scripts
shellcheck <script>

# Unused Rust dependencies
cargo shear
```

Treat Clippy warnings as issues to fix, not as pre-existing noise.

The root `clippy.toml` configures PV-specific lint behavior. It intentionally disallows direct use of risky primitives such as raw filesystem methods, raw process spawning, raw environment access, direct terminal printing, `dbg!`, and unsafe zero-initialization.

Use PV helpers instead, so permissions, atomic writes, injected test homes, process ownership checks, structured output, and diagnostics stay consistent.

`clippy.toml` configures these lint details, and the Cargo workspace enables the policy through `[workspace.lints.clippy]`. Every workspace package should opt in with `[lints] workspace = true`, so CI and local checks only need `-D warnings`.

If a lint must be silenced, prefer `#[expect(...)]` over `#[allow(...)]`, and keep the reason local and specific.

For example, a filesystem helper that intentionally wraps direct `std::fs` calls should use a narrow expectation on the smallest possible item:

```rust
#[expect(
    clippy::disallowed_methods,
    reason = "PV filesystem helper owns direct filesystem access"
)]
fn write_atomically(...) {
    // ...
}
```

Do not add broad Clippy ignores for convenience.

## Crate structure

Rust does not allow circular dependencies between crates. To visualize the crate hierarchy, install
[cargo-depgraph](https://github.com/jplatte/cargo-depgraph) and graphviz, then run:

```shell
cargo depgraph --dedup-transitive-deps --workspace-only | dot -Tpng > graph.png
```
