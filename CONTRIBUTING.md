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
