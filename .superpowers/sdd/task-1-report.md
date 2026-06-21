What you implemented

- Added the shared bundled PHP defaults asset at `crates/resources/src/php-defaults.ini` using the exact stripped values from the task brief.
- Added `crates/resources/src/php_defaults.rs` with:
  - `PHP_TRACK_DEFAULT_INI`
  - `PhpTrackDefaults { etc_dir, php_ini, conf_dir }`
  - `php_track_defaults(&PvPaths, &str) -> PhpTrackDefaults`
  - `ensure_php_track_defaults(&PvPaths, &str) -> Result<PhpTrackDefaults, StateError>`
  - `php_track_environment(&PvPaths, &str) -> BTreeMap<String, String>`
  - `php_track_exec_environment(&PvPaths, &str) -> Vec<(OsString, OsString)>`
- Exported the new API from `crates/resources/src/lib.rs`.
- Added focused integration tests in `crates/resources/tests/php_defaults.rs` for one-time seeding, blocking-path rejection, and environment helper output.

What you tested and results

- Ran `cargo nextest run -p resources -E 'test(php_track_defaults_)'`.
- Result: 3 tests passed, 0 failed.

TDD Evidence: RED command/output and GREEN command/output

RED command:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

RED output:

```text
error[E0432]: unresolved imports `resources::PHP_TRACK_DEFAULT_INI`, `resources::ensure_php_track_defaults`, `resources::php_track_defaults`, `resources::php_track_environment`, `resources::php_track_exec_environment`
 --> crates/resources/tests/php_defaults.rs:6:5
  |
6 |     PHP_TRACK_DEFAULT_INI, ensure_php_track_defaults, php_track_defaults,
  |     ^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^ no `php_track_defaults` in the root
  |     |                      |
  |     |                      no `ensure_php_track_defaults` in the root
  |     no `PHP_TRACK_DEFAULT_INI` in the root
7 |     php_track_environment, php_track_exec_environment,
  |     ^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^^ no `php_track_exec_environment` in the root
  |     |
  |     no `php_track_environment` in the root

For more information about this error, try `rustc --explain E0432`.
error: could not compile `resources` (test "php_defaults") due to 1 previous error
error: command `/Users/clovismuneza/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test --no-run --message-format json-render-diagnostics --package resources` exited with code 101
```

GREEN command:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

GREEN output:

```text
Finished `test` profile [unoptimized + debuginfo] target(s) in 5.04s
────────────
 Nextest run ID 658696da-43e8-486a-8b87-c0c55ca1d59a with nextest profile: default
    Starting 3 tests across 8 binaries (111 tests skipped)
        PASS [   0.013s] (1/3) resources::php_defaults php_track_defaults_env_helpers_point_at_track_etc
        PASS [   0.014s] (2/3) resources::php_defaults php_track_defaults_reject_blocking_paths
        PASS [   0.015s] (3/3) resources::php_defaults php_track_defaults_seed_stripped_sample_once
────────────
     Summary [   0.016s] 3 tests run: 3 passed, 111 skipped
```

Files changed

- `crates/resources/src/php-defaults.ini`
- `crates/resources/src/php_defaults.rs`
- `crates/resources/src/lib.rs`
- `crates/resources/tests/php_defaults.rs`
- `.superpowers/sdd/task-1-report.md`

Self-review findings

- The implementation is intentionally narrow: it seeds the per-track `etc/php.ini` once, creates `conf.d`, and exposes env helpers without adding validation or cross-crate behavior not required by this task.
- Blocking `conf.d` or `etc` paths return a `StateError::Filesystem` with a task-specific message so later callers can surface a clear failure.
- The root sample file `php.ini` was not modified or tracked.

Any concerns

- None for Task 1 scope.

Fix follow-up from review

What changed

- Completed the bundled defaults asset tail after `[soap]` with the required active SOAP cache settings plus `[sysvshm]`, `[ldap]`, `[dba]`, `[opcache]`, `[curl]`, `[openssl]`, and `[ffi]` in the required order.
- Tightened `ensure_php_track_defaults` to:
  - reject unsupported tracks outside `8.3`, `8.4`, and `8.5`
  - validate an existing `php.ini` is a regular file
  - validate an existing `php.ini` is readable by attempting to read it
- Changed `crates/resources/src/lib.rs` to `pub mod php_defaults;` to match the brief.
- Expanded the focused integration tests to cover:
  - exact required asset tail content/order
  - unsupported-track rejection
  - blocking `php.ini` directory rejection

Review-fix TDD evidence

RED command:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

RED output:

```text
────────────
 Nextest run ID 89d9113d-b0f1-4f1c-8bab-ae38f26eeb38 with nextest profile: default
    Starting 5 tests across 8 binaries (111 tests skipped)
        PASS [   0.013s] (1/5) resources::php_defaults php_track_defaults_env_helpers_point_at_track_etc
        FAIL [   0.014s] (2/5) resources::php_defaults php_track_defaults_reject_blocking_php_ini_paths
Error: expected blocking php.ini path to fail
        FAIL [   0.015s] (3/5) resources::php_defaults php_track_defaults_reject_unsupported_tracks
Error: expected unsupported PHP track to fail
        FAIL [   0.015s] (4/5) resources::php_defaults php_track_defaults_seed_stripped_sample_once
assertion failed: PHP_TRACK_DEFAULT_INI.ends_with(...)
        PASS [   0.016s] (5/5) resources::php_defaults php_track_defaults_reject_blocking_paths
────────────
     Summary [   0.016s] 5 tests run: 2 passed, 3 failed, 111 skipped
```

GREEN command:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

GREEN output:

```text
Finished `test` profile [unoptimized + debuginfo] target(s) in 0.85s
────────────
 Nextest run ID 5c7f6d68-cd55-4a9e-8578-d160b28c8bb1 with nextest profile: default
    Starting 5 tests across 8 binaries (111 tests skipped)
        PASS [   0.008s] (1/5) resources::php_defaults php_track_defaults_reject_unsupported_tracks
        PASS [   0.008s] (2/5) resources::php_defaults php_track_defaults_env_helpers_point_at_track_etc
        PASS [   0.009s] (3/5) resources::php_defaults php_track_defaults_reject_blocking_paths
        PASS [   0.009s] (4/5) resources::php_defaults php_track_defaults_reject_blocking_php_ini_paths
        PASS [   0.009s] (5/5) resources::php_defaults php_track_defaults_seed_stripped_sample_once
────────────
     Summary [   0.010s] 5 tests run: 5 passed, 111 skipped
```

Files changed for review fixes

- `crates/resources/src/php-defaults.ini`
- `crates/resources/src/php_defaults.rs`
- `crates/resources/src/lib.rs`
- `crates/resources/tests/php_defaults.rs`
- `.superpowers/sdd/task-1-report.md`

Self-review for fixes

- The new track gate is enforced at the seeding entrypoint, which is where arbitrary-track mutation could occur.
- Existing `php.ini` validation now fails fast for non-files and unreadable files, while preserving the existing file content when valid.
- The root `/Users/clovismuneza/Apps/pv/php.ini` remained unchanged and untracked.

Review-fix second pass: strict public helpers and symlink rejection

What changed

- Changed `php_track_defaults`, `php_track_environment`, and `php_track_exec_environment` to return `Result<..., StateError>` and validate the PHP track before constructing paths or env overlays.
- Kept default path construction behind a private helper that is called only after supported-track validation.
- Changed existing `php.ini` validation to use `state::fs::path_entry_exists` and `state::fs::path_is_file`, which are based on `symlink_metadata`, so symlinked `php.ini` paths are rejected instead of followed.
- Added integration coverage for unsupported public helper APIs and symlinked `php.ini` rejection.

TDD RED command:

```shell
cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

TDD RED output:

```text
Compiling resources v0.1.3 (/Users/clovismuneza/Apps/pv/crates/resources)
error[E0277]: the `?` operator can only be applied to values that implement `Try`
  --> crates/resources/tests/php_defaults.rs:79:20
   |
79 |     let defaults = php_track_defaults(&paths, "8.5")?;
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ the `?` operator cannot be applied to type `PhpTrackDefaults`

error[E0308]: mismatched types
   --> crates/resources/tests/php_defaults.rs:148:26
    |
148 |     assert_invalid_track(php_track_defaults(&paths, "8.2"), "8.2")?;
    |                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `Result<_, StateError>`, found `PhpTrackDefaults`

error[E0308]: mismatched types
   --> crates/resources/tests/php_defaults.rs:149:26
    |
149 |     assert_invalid_track(php_track_environment(&paths, "8.2"), "8.2")?;
    |                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `Result<_, StateError>`, found `BTreeMap<String, String>`

error[E0308]: mismatched types
   --> crates/resources/tests/php_defaults.rs:150:26
    |
150 |     assert_invalid_track(php_track_exec_environment(&paths, "8.2"), "8.2")?;
    |                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `Result<_, StateError>`, found `Vec<(OsString, OsString)>`

error: could not compile `resources` (test "php_defaults") due to 8 previous errors
```

TDD GREEN command:

```shell
cargo fmt --all && cargo nextest run -p resources -E 'test(php_track_defaults_)'
```

TDD GREEN output:

```text
Finished `test` profile [unoptimized + debuginfo] target(s) in 0.75s
────────────
 Nextest run ID a7022687-cad4-4125-bd8e-48188c296f20 with nextest profile: default
    Starting 7 tests across 8 binaries (111 tests skipped)
        PASS [   0.015s] (1/7) resources::php_defaults php_track_defaults_reject_unsupported_tracks
        PASS [   0.015s] (2/7) resources::php_defaults php_track_defaults_helpers_reject_unsupported_tracks
        PASS [   0.015s] (3/7) resources::php_defaults php_track_defaults_env_helpers_point_at_track_etc
        PASS [   0.017s] (4/7) resources::php_defaults php_track_defaults_reject_blocking_paths
        PASS [   0.018s] (5/7) resources::php_defaults php_track_defaults_reject_blocking_php_ini_paths
        PASS [   0.018s] (6/7) resources::php_defaults php_track_defaults_seed_stripped_sample_once
        PASS [   0.018s] (7/7) resources::php_defaults php_track_defaults_reject_symlinked_php_ini_paths
────────────
     Summary [   0.019s] 7 tests run: 7 passed, 111 skipped
```

Files changed

- `crates/resources/src/php_defaults.rs`
- `crates/resources/tests/php_defaults.rs`
- `.superpowers/sdd/task-1-report.md`

Self-review findings

- Unsupported PHP tracks now fail before any public helper can synthesize defaults paths or env overlays.
- `ensure_php_track_defaults` still preserves a valid existing seeded `php.ini`, but rejects directories and symlinks before checking readability.
- The implementation uses existing `state::fs` helpers for symlink-aware filesystem checks and does not add panic, assert, unwrap, unsafe code, or clippy ignores.
- The root `/Users/clovismuneza/Apps/pv/php.ini` sample remained untouched and untracked.

Any concerns

- No functional concerns. I did not add a chmod-based unreadable-file fixture because local macOS permission behavior can make that unreliable; readable-file validation is still exercised by the implementation through `state::fs::read_to_string`.
