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
holding every test's duration and output. Native Linux daemon registration
tests have a separate `nextest-system-run-<runner>` artifact. CI pins nextest
0.9.146 because the recording format is experimental, so use the same version
locally. To see where time goes, download a recording and open its trace in
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
lanes run the full workspace suite, including native process inspection,
monitors, and daemon IPC.
Linux Clippy already covers every target and feature. macOS behavior remains
covered by the existing native matrix.

Build the Ubuntu 22.04 development image on the host's native architecture:

```shell
df -h /
docker build -f scripts/linux/builder.Dockerfile -t pv-linux-builder .
pv_mount_source=${PWD//\"/\"\"}
docker run --rm --init --mount "type=bind,\"src=$pv_mount_source\",dst=/src,readonly" \
  --mount type=volume,src=pv-linux-target,dst=/target \
  --env CARGO_TARGET_DIR=/target --env CARGO_BUILD_JOBS=1 \
  --env CARGO_PROFILE_DEV_DEBUG=0 --env CARGO_INCREMENTAL=0 --workdir /src pv-linux-builder \
  cargo nextest run --profile ci --workspace --all-features --locked
```

The builder runs tests as an ordinary user so filesystem permission checks
remain meaningful. It includes stable Rust, Clippy, rustfmt, nextest 0.9.146,
cargo-shear, shellcheck, and the native build tools. Its Ubuntu base is pinned
by digest and supplies the glibc 2.35 baseline for Linux artifacts. Local Apple
Silicon runs are ARM64 only; use native GitHub runners for amd64.

The MySQL packaging fixture requires the baseline builder's CMake and library
layout. CI runs it explicitly on both CPUs, alongside the full native workspace
and system tests. Inside that builder, select it with:

```shell
cargo nextest run --profile ci --locked -p pv-release --all-features --run-ignored only \
  -E 'test(=mysql_linux_build_packages_relocatable_elf_and_rejects_newer_glibc)'
```

Use Docker's `--init` for ordinary builder test runs so orphaned children are
reaped, as they are by init on a normal host. The systemd test containers below
already run their own init.
Linux CI and this command omit debug symbols and incremental build caches to
keep the full workspace build within runner disk space; debug assertions stay enabled.

Before seeding unpublished artifacts for Linux acceptance, run this check inside
the builder. It verifies that the reserved `.invalid` manifest host produces a
transport error and PV uses its cached manifest. It refuses proxy environments
so a proxy connection failure cannot stand in for direct DNS resolution.

```shell
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy \
  cargo nextest run --profile ci --locked -p resources --all-features --run-ignored only \
  -E 'test(=invalid_manifest_host_uses_transport_error_and_cached_manifest)'
```

Ignored daemon artifact contracts can use unpublished local inputs. Put the
generated `manifest.json` and archives in one directory, named by the filenames
in their URLs, then set `PV_E2E_ARTIFACT_DIR` for the test process:

```shell
PV_E2E_REAL_ARTIFACTS=1 PV_E2E_ARTIFACT_DIR=/path/to/local-artifacts \
  PV_E2E_ARTIFACT_MANIFEST_URL=https://artifacts.pv-linux.invalid/manifest.json \
  cargo nextest run --profile ci --locked -p daemon --all-features --run-ignored only \
  -E 'test(=real_caddy_satisfies_the_gateway_contract)'
```

The test transport reads those files locally. Missing inputs fail without a
network fallback; the normal downloader still checks archive SHA-256 hashes.
Without `PV_E2E_ARTIFACT_DIR`, the contracts use their existing HTTP client.
This environment variable is test-only and does not configure the installed app.

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

For a complete Linux support run, first dispatch `artifact-recipes.yml` on the
Linux branch with `resource=all`, `track=all`, and `platform=all-linux`. After
that build succeeds, dispatch `linux-acceptance.yml` on the same stack with its
`source_run_id`. These workflows build candidate files and upload test outputs;
they do not publish releases or push builder images to a registry. Acceptance
checks all resource tracks on Ubuntu 22.04/24.04/26.04, Debian 12/13 and Fedora
43/44 for both native CPUs, plus native Ubuntu and Alpine installer refusal.
Each pair uploads its result and step logs. The acceptance script runs only as
an ordinary user on an explicitly opted-in disposable machine, with its own
user manager and no existing PV installation.


The daemon-registration system test requires a disposable user's real user
manager and refuses an existing PV home, unit file, or loaded service. Build PV
and the daemon examples first, then run as that user with its session-bus
environment intact:

```shell
cargo build --locked -p pv -p daemon --examples --bins
PV_LINUX_SYSTEM_TESTS=1 PV_LINUX_SYSTEM_PV="$PWD/target/debug/pv" \
  cargo nextest run --profile ci --locked -p cli -p daemon --run-ignored only \
  -E 'test(=systemd_user_registration_preserves_monitors_and_refuses_foreign_units)'
```

This test installs its own real daemon and a fake Redis runtime. A test-only
150-second user timer stops them if the test is killed; successful cleanup
cancels it. The ordinary workspace suite compiles this test but leaves it ignored.

The helper lifecycle system test requires a disposable **machine**, passwordless
sudo for its non-root account, and no existing helper installation or units. It
refuses to run outside the dedicated bounded service below. Build the helper and
compile the CLI tests first, then run as that account:

```shell
cargo build --locked -p pv-privileged-helper
cargo nextest list --locked -p cli
sudo -n systemd-run --wait --pipe --collect --unit=pv-helper-system-test \
  --uid="$(id -u)" --property=KillMode=control-group \
  --property=RuntimeMaxSec=120s --property=TimeoutStopSec=5s \
  --working-directory="$PWD" --setenv="HOME=$HOME" --setenv="PATH=$PATH" \
  --setenv=PV_LINUX_SYSTEM_TESTS=1 \
  --setenv="PV_LINUX_SYSTEM_HELPER=$PWD/target/debug/pv-helper" \
  "$(command -v cargo)" nextest run --profile ci --locked -p cli --run-ignored only \
  -E 'test(=systemd_helper_lifecycle_authenticates_replaces_rolls_back_and_removes)'
```

The test also verifies that sudo children remain in its service cgroup. It checks
native socket activation, replacement, peer and lifecycle owner
authentication, rollback, and repeat removal. Its root timer fires after 150
seconds if cleanup did not finish. It requests stop for the test and helper
units, kills remaining service processes, waits for the units to stop, then
removes fixture files.
Normal cleanup cancels that timer after verifying removal. Failed cleanup keeps
any scheduled timer and the root-owned `/run/pv-helper-system-test` directory for
inspection. These live checks are ignored by the ordinary suite. The two ignored
`platform` helper checks for root file handling and temporary credential changes
need only an ordinary disposable root container; they do not require systemd.

The DNS system test uses that same bounded `pv-helper-system-test` service and
helper build. It additionally requires active systemd-resolved, only
`nameserver 127.0.0.53` in `/etc/resolv.conf`, no existing PV DNS unit or `pv0`,
and active NetworkManager on Fedora. Select it in the command above with:

```shell
-E 'test(=dns_system::systemd_dns_routes_test_reapplies_after_restart_changes_port_and_refuses_conflicts)'
```

It runs the real daemon's DNS resolver from a temporary PV home, verifies
fresh `getaddrinfo` through resolved immediately after installation, resolved restart and a DNS
port change, and checks that foreign units, links and `~test` routes survive
rejected mutations.
Its independent 140-second root timer stops the test and helper/DNS services,
removes only preflight-absent fixture paths and links, and restores resolved.
Explicit cleanup cancels that timer only after verifying removal. Failed cleanup
keeps `/run/pv-dns-system-test` and any scheduled timer for inspection.
CI runs each helper and DNS case in its own bounded service and nextest state
directory. The `nextest-helper-run-<runner>` artifact contains one recording ZIP
per case, including a failed case when recording export succeeds.
Nextest serializes helper-system tests within a run because they use the
same fixed installation paths. Run only one such invocation per disposable machine.

The socket-proxy system test uses the same bounded service and requires free
loopback ports 80 and 443. Build the daemon examples with
`cargo build --locked -p daemon --examples`, then select:

```shell
-E 'test(=ports_system::systemd_socket_proxies_route_gateway_identity_repair_and_refuse_foreign_units)'
```

It runs a monitored native fake Caddy with a real test CA, verifies HTTP and HTTPS
Gateway identity through the proxies, and checks repair, foreign ownership,
repeated removal, and the limits of loaded socket and command metadata after
reload. Its independent 140-second timer stops accepted helper work before
removing only preflight-absent fixture units and files. Nextest serializes all
tests in this helper-system binary; separate invocations must also remain serial.

## Crate structure

Rust does not allow circular dependencies between crates. To visualize the crate hierarchy, install
[cargo-depgraph](https://github.com/jplatte/cargo-depgraph) and graphviz, then run:

```shell
cargo depgraph --dedup-transitive-deps --workspace-only | dot -Tpng > graph.png
```
