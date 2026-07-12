# CI Performance Design

## Summary

PV's pull-request CI will keep one macOS job and the full application test suite while removing redundant work and fixing test-fixture defects that make the suite wait unnecessarily. The change will:

- remove blanket serialization of daemon tests,
- fix Redis and RustFS fixture shutdown behavior,
- make the immediate-exit Mailpit fixture track the process that actually exits,
- add the repository's existing pinned Rust cache action,
- remove the internal Rust documentation gate, and
- move artifact-recipe CLI smoke coverage into the existing integration test and remove its standalone workflow invocation.

Formatting, Clippy, unused-dependency checks, recipe shellcheck, and all non-ignored workspace tests remain required. Production shutdown behavior and its ten-second grace period remain unchanged.

## Evidence And Root Causes

The cited [GitHub Actions job](https://github.com/prvious/pv/actions/runs/29180358045/job/86616706422) ran for pull-request commit `d3ebe255f2de5bbeeb3aa8362e1f15ef417b1aa5` on `macos-14` and took 7 minutes 4 seconds. Its material steps were:

| Step | Duration |
| --- | ---: |
| Run tests | 4m 32s |
| Run Clippy | 54s |
| Build docs | 50s |
| Validate artifact recipe metadata and fixtures | 20s |

The test step spent 2 minutes 8 seconds compiling and 141.910 seconds executing 1,003 tests.

### Blanket daemon-test serialization

`.config/nextest.toml` puts every test in the `daemon` package, plus tests whose names match `daemon_`, into a group with one worker. That filter matched 254 tests in the cited run. Those tests accounted for approximately 136.465 of the suite's 141.910 aggregate execution seconds, so the configuration serialized nearly all meaningful test work.

The group was introduced after a daemon fixture race. A subsequent fix made accepted diagnostic streams blocking, addressing the underlying fixture behavior. A resource audit found isolated temporary homes, databases, sockets, and ports in current daemon tests rather than a shared resource that requires package-wide serialization.

Three complete local runs with the group disabled passed all 1,003 tests. Their nextest execution summaries were 37.885, 38.694, and 39.370 seconds. They used an empty temporary config with `cargo nextest run --config-file /tmp/pv-nextest-empty.toml --workspace --all-features --locked` after the initial compile on macOS.

### Fixture shutdown deadlocks

The Redis and RustFS fake servers call Python's `BaseServer.shutdown()` from signal handlers on the same thread that is running `serve_forever()`. Python requires `shutdown()` to be called from a different thread. The fixtures therefore wait until PV's intentional ten-second graceful-shutdown deadline and are force-killed.

Seven Redis and RustFS tests consumed approximately 89 seconds in the cited serialized run. The tests cover supported Managed Resource behavior and are valuable; the fake servers are the defect.

### Immediate-exit process race

The fast-exit Mailpit fixture launches Python from a shell script. The Python child exits immediately after serving readiness, but PV tracks the shell PID. Depending on how quickly the shell reaps its child, the runtime can appear alive during PV's post-readiness check. This makes `demanded_resource_cleans_runtime_files_when_process_exits_after_readiness` timing-dependent even though the behavior it protects is meaningful.

### Redundant CI gates

The Rust documentation command validates rustdoc-specific warnings, including broken intra-doc links, for internal crate documentation. PV is an unpublished application and its workspace crates are internal. The command does not build user-facing Markdown documentation or run doctests, and neither `DESIGN.md`, `CONTEXT.md`, nor an ADR establishes internal rustdoc as a product or release artifact. Removing it deliberately stops treating internal rustdoc correctness as a pull-request requirement; Clippy and nextest are not equivalent replacements for that narrow coverage.

The standalone artifact-recipe step invokes `pv-release` twice with committed recipe paths. `crates/pv-release/tests/recipe_fixtures.rs` covers the same inputs and generation behavior, while parser tests cover the command arguments, but those tests currently bypass the binary's dispatch arms. Before removing the workflow step, the existing recipe integration test will invoke the compiled `pv-release` executable for both commands and validate the resulting archives, records, and manifest as it does today. This preserves automatic CLI parsing, dispatch, path wiring, and output coverage without a separate Cargo invocation. The manual artifact workflow continues running the commands against real builds, and recipe scripts still require shellcheck in pull-request CI.

### Missing compilation cache

The CI job has no Cargo cache even though `.github/workflows/real-artifact-e2e.yml` already uses `Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32`. The cited run downloaded hundreds of crates and compiled test artifacts from scratch.

## Goals

- Reduce pull-request CI wall time materially from the cited seven-minute run.
- Keep the complete non-ignored workspace application test suite.
- Preserve checks that directly protect source quality or supported product behavior.
- Fix slow and flaky fixtures at their cause instead of lowering production timeouts.
- Keep CI simple enough to understand from one workflow file.
- Leave release-only and privileged validation in their existing dedicated workflows.

## Non-Goals

- Do not change PV's production resource-stop grace period.
- Do not remove individual application tests.
- Do not weaken Clippy, formatting, dependency, or shell-script validation.
- Do not add Linux CI; PV v1 remains macOS-only.
- Do not split the workflow into multiple jobs that repeat setup and compilation.
- Do not alter real-artifact, privileged release-candidate, or artifact-publication workflows.
- Do not change workflow triggers, the Rust toolchain policy, or platform matrices.

## CI Job Design

The workflow remains a single `macos-14` job so every Rust command reuses the same target directory and cache. Its steps will be:

1. Check out the repository.
2. Install Rust.
3. Restore/save Cargo artifacts with `Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32`, the exact revision already vetted in `real-artifact-e2e.yml`.
4. Install cargo-nextest, cargo-shear, and shellcheck.
5. Check formatting.
6. Run workspace Clippy for all targets and features with the lockfile enforced and warnings denied.
7. Check unused dependencies.
8. Shellcheck artifact recipe scripts.
9. Run the complete non-ignored workspace suite with nextest.

The current `Build docs` step will be removed. `Validate artifact recipe metadata and fixtures` will be removed only after its binary-level coverage has been folded into the retained integration test.

The cache is an acceleration only. A cache miss must still produce a correct run, and no generated output becomes a source of truth.

## Test Scheduling Design

Delete `.config/nextest.toml` so nextest uses its normal scheduler for the workspace. No replacement group will be added speculatively.

If a future test demonstrates a concrete shared-resource collision, the narrowest exact test set may receive an isolated group together with a comment documenting the resource. Package-wide or name-prefix serialization must not be reintroduced without evidence that the entire selected set conflicts.

## Fixture Design

### Redis

The fake Redis server will bind and run `serve_forever()` on the main thread exactly as it does today. Its `SIGTERM` and `SIGINT` handler will start a short-lived helper thread that calls `shutdown()`, then return to the server loop. The helper requests shutdown from a different thread, satisfying Python's threading contract; `serve_forever()` then returns normally on the main thread.

Keeping the server loop on the main thread means bind failures and unexpected loop termination continue to terminate the fixture rather than leaving a main thread waiting indefinitely. The fixture's protocol behavior, arguments, data-directory behavior, and port allocation remain unchanged.

### RustFS

The fake RustFS API server will remain in `serve_forever()` on the main thread, and the console server will remain on its existing worker thread. The signal handler will start a helper thread that requests API shutdown and then return to the API loop. After the API loop returns, the main thread will shut down the console server from outside its worker thread and exit. This avoids both same-thread deadlock and a new failure mode where all server loops could stop while the main thread waits indefinitely.

The S3 behavior, credential recording, API and console addresses, object persistence, and rejection fixture remain unchanged.

### Immediate-exit Mailpit

The fast-exit fixture will be a directly executable Python script with a `#!/usr/bin/env python3` shebang instead of a shell script that launches Python through standard input. It will preserve the fake adapter's `[smtp_port, dashboard_port]` argument contract, read the dashboard port from `sys.argv[2]`, serve one successful readiness response, flush it, and exit immediately.

PV will therefore track the PID of the process that exits. The executable's script path remains present in the command line so the supervisor's ownership checks continue to recognize it. Blindly changing the shell to `exec python3 -` is not acceptable because the standard-input marker would replace the script path in the relevant command-line positions.

## Coverage Decisions

No application test will be removed. The Redis, RustFS, and immediate-exit tests exercise real Managed Resource lifecycle, allocation, credentials, reconciliation, and cleanup behavior described by PV's design.

The standalone recipe command step will be consolidated rather than simply discarded:

- the existing `pv-release` recipe integration test will use the repository's established `env!("CARGO_BIN_EXE_pv-release")` and `std::process::Command` pattern to run `generate-recipe-fixtures` and `generate-manifest`,
- the test will pass the exact committed recipe paths and require both commands to exit successfully,
- its existing archive, record, manifest, and snapshot assertions will validate the generated output,
- recipe scripts remain covered by shellcheck, and
- actual recipe commands remain a gate in the manual artifact build workflow.

Removing rustdoc intentionally permits internal documentation-only warnings, including broken internal links, to stop blocking pull requests. Internal rustdoc should return as a gate if PV introduces a publishable Rust API, doctests that form part of application verification, or an explicit documentation requirement. None exists today.

## Error Handling And Safety

- Fixture changes must not catch or suppress bind, startup, or protocol errors.
- Signal handlers must delegate `shutdown()` to a different thread, return to the main server loop, and allow the process to exit normally.
- Unexpected termination of a main server loop must terminate the fixture rather than leave a waiting process behind.
- Test cleanup must continue removing runtime PID, metadata, and configuration files.
- Production process supervision, shutdown escalation, and timeout values remain untouched.
- Cache misses and invalid cache entries must fall back to normal compilation through the cache action's standard behavior.

## Verification

Implementation is complete only after all of the following pass:

1. Run the affected Redis, RustFS, and immediate-exit lifecycle tests individually.
2. Repeat the affected fixture tests to exercise shutdown and process-exit timing.
3. Add and run a focused macOS supervisor test that starts a directly executable Python shebang script, confirms `verify_ownership()` recognizes the live process, and stops it cleanly.
4. Update and run the `pv-release` recipe-fixtures integration test, confirming that it invokes the compiled binary for both removed workflow commands and retains its output assertions.
5. Run the complete workspace nextest suite without serialization at least three times; every run must pass all non-ignored tests.
6. Run `cargo fmt --all --check`.
7. Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
8. Run `cargo shear`.
9. Run the artifact recipe shellcheck command retained by CI.
10. Run `git diff --check` and inspect the final workflow ordering and action revision.

The existing lifecycle tests remain the regression checks for fixture behavior and must keep their current assertions. Two narrow coverage changes are required: the recipe integration test will absorb the removed binary smoke commands, and the ownership test will protect the interpreter-script command-line shape introduced by the fast-exit fixture.

## Expected Outcome

Removing the blanket group should reduce nextest execution from approximately 142 seconds to roughly 40 seconds on comparable hardware before considering cache improvements. Correct fixture shutdown removes repeated ten-second waits and prevents orphan fixture processes. Removing rustdoc and the standalone recipe commands eliminates about 70 seconds from the cited cold run while the recipe commands continue running inside the already-built test suite.

The expected GitHub Actions wall time is approximately two to three minutes on cache hits and nearer four minutes after a cold cache miss. These are operational targets rather than hard test thresholds because runner and network performance vary. The correctness acceptance criterion is unchanged: every retained gate and all non-ignored workspace tests must pass.
