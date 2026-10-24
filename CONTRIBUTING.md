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

Some tests require `python3` on `PATH`: two supervisor process-group tests, and `pv-release`'s installer and smoke tests. They use only Python's standard library; no Python packages or virtual environment is required.

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

Tests that start long-running Managed Resource or Gateway runtimes run them
under the test monitor. Fixture setup calls `pv_fake::install_monitor(paths)`.
That links the daemon's `pv-monitor` example into the test's PV home as its
`pv` binary and gives the monitors the test process's lifeline. Capture
assertions while the runtime is live, then use explicit cleanup where the guard
exposes it so cleanup failures fail the test. If an operation has already
failed, preserve that primary failure while also reporting cleanup failures.
Cleanup goes through `stop_recorded_runtimes` or
`ProcessSupervisor::stop_subject`. The runtime's monitor stops its whole
process group and waits for the members to exit. PV removes the records only
after that verified stop.

Fixture guards also clean up from `Drop` where they can, so normal return,
early return, panic unwind, and cancellation cannot bypass teardown. The
fallback uses an independent cleanup runtime and reports failures, because
`Drop` cannot return them to the test. Whenever the test process exits, even by
SIGKILL, its lifeline closes and every monitor it started stops its runtime's
process group and exits. If a monitor can't prove the group is gone, it keeps its
records for recovery, so check for leftover processes after an unexplained
failure.

Long-running fixtures that stand in for a runtime use `pv-fake`, not a script.
`pv_fake::install(executable, persona)` places a native fake at the
runtime's executable path. The fake exits when the test process that installed
it dies, SIGKILL included, so it needs no parent watcher. It records its
lifecycle in `<executable>.pv-fake.events.jsonl`. `cargo nextest run` builds it
as the daemon's `pv-fake` example, but runs narrowed with `--lib` or
`--test <name>` do not; filter with `-E` instead. A fake from an older build
refuses to start and says how to rebuild. Explicit cleanup still applies. To stop a fake at a known point, pass `FakeSettings` to
`pv_fake::install_with_settings` and wait on its events, such as `held` and
`descendant_spawned`, instead of marker files. Steer a running Gateway fake
with `pv_fake::write_gateway_control`, which rejects unknown keys.

Tests must not reach the PV installed on the machine running them, whose pf rules
send loopback ports `80` and `443` to its real Gateway. A fixture that installs
a fake Caddy calls `daemon::gateway::use_unrouted_public_probe_ports_for_test()`
first, so Gateway readiness never probes those ports. Daemons started with
`RunningDaemon::start_without_managed_resource_adapters` fetch the artifact
manifest from a closed loopback port, so the fetch fails at once.

A port a test hands to a fake, a runtime, or `pv.db` comes from
`pv_fake::port()` (or `pv_fake::ports::<N>()` for several), never from binding
port 0 and closing the listener, which lets another test take the port before it
is used. The pool's ports are below every OS's outgoing-connection range and
away from PV's own ports, and each block is held under a file lock, so
concurrent test runs, including runs in other worktrees, never get the same
port. A test that starts a daemon first calls `pv_fake::seed_dns_port(paths)`: a
restart rebinds the saved DNS port without retrying, and a first start gives up
after 10 tries when many tests start daemons at once.

Shell scripts remain only where a test needs what the fake doesn't do: the
process-group tests, which need a child that outlives its parent or ignores
SIGTERM. A script that stays alive must give up on its own, so a test that dies
before stopping it leaves nothing running for long, but not before the CI
profile's 120-second test limit, so a stalled test can't pass because its
fixture already exited; the supervisor tests' `IDLE_SHELL_LOOP` gives up after
150 seconds. When a test reads a PID a script reports, make sure it can't read
it half written: have the script write it to a temporary file and rename it
into place, or wait for a later signal, such as the complete line or a ready
file written afterwards.

Use the CI nextest profile for CI runs (`cargo nextest run --profile ci`). It
warns after 60 seconds, terminates a wedged test after 120 seconds, and allows
10 seconds for graceful exit. The default profile remains available for tests
with intentionally longer limits.

CI records each test run and uploads it as a `nextest-run-<runner>` artifact
holding every test's duration and output. CI pins nextest 0.9.146 because the
recording format is experimental, so use the same version locally. To see where
time goes, download a recording and open its trace in
[Perfetto](https://ui.perfetto.dev):

```shell
gh run download <run-id> -n nextest-run-macos-15-intel
cargo nextest store export-chrome-trace nextest-run.zip -o trace.json
```

`NEXTEST_EXPERIMENTAL_RECORD=1 cargo nextest replay -R nextest-run.zip` replays
the run in the terminal.

Runtime cleanup must never discover ownership through process-name scans or
signal a PID found only in a record. A runtime is stopped through its monitor.
If the monitor has died, recovery signals the runtime's process group only
while the runtime identity and boot session in the monitor's record still
match. A test may signal and reap a child it directly spawned and still owns. These
guarantees do not cover machine or kernel failure, a test process that is
stopped rather than killed (its fakes keep running), or descendants that
deliberately escape their owned process group.

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

## Linux development

Linux system integration is being implemented in ordered slices. The Linux CI
lanes currently run the portable suites and explicit unsupported boundaries;
the full workspace suite will be enabled with Linux monitors and daemon IPC.
Linux Clippy already covers every target and feature. macOS behavior remains
covered by the existing native matrix.

Build the Ubuntu 22.04 development image on the host's native architecture:

```shell
df -h /
docker build -f scripts/linux/builder.Dockerfile -t pv-linux-builder .
pv_mount_source=${PWD//\"/\"\"}
docker run --rm --mount "type=bind,\"src=$pv_mount_source\",dst=/src,readonly" \
  --mount type=volume,src=pv-linux-target,dst=/target \
  --env CARGO_TARGET_DIR=/target --env CARGO_BUILD_JOBS=1 --workdir /src pv-linux-builder \
  cargo nextest run --profile ci --locked -p resources -p self-update
```

The builder runs tests as an ordinary user so filesystem permission checks
remain meaningful. It includes stable Rust, Clippy, rustfmt, nextest 0.9.146,
cargo-shear, shellcheck, and the native build tools. Its Ubuntu base is pinned
by digest and supplies the glibc 2.35 baseline for Linux artifacts. Local Apple
Silicon runs are ARM64 only; use native GitHub runners for amd64.

System integration needs a separate disposable container with systemd as PID 1:

```shell
df -h /
docker build -f scripts/linux/test.Dockerfile --build-arg BASE=ubuntu:22.04 \
  -t pv-linux-test:ubuntu-22.04 .
bash scripts/linux/container.sh up ubuntu-22.04
bash scripts/linux/container.sh exec ubuntu-22.04 bash -c 'id; systemctl --user status'
bash scripts/linux/container.sh down ubuntu-22.04
```

This uses privileged Docker containers, a private cgroup namespace, and a
read-only source mount. It never runs setup against the host's PV installation.
The helper labels containers with their worktree and refuses to execute in or
remove another worktree's container. Linger is enabled only for the container's
test user. It is never part of PV's installed daemon behavior.

Run `python3 scripts/linux/test_container.py` and `shellcheck scripts/linux/*.sh`
when changing the helper. Distro acceptance is a separate gate; a successful
build or unit suite does not establish supported Linux distributions.

## Crate structure

Rust does not allow circular dependencies between crates. To visualize the crate hierarchy, install
[cargo-depgraph](https://github.com/jplatte/cargo-depgraph) and graphviz, then run:

```shell
cargo depgraph --dedup-transitive-deps --workspace-only | dot -Tpng > graph.png
```
