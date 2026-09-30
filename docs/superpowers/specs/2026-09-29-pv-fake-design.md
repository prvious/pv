# pv-fake Test Service Design

Status: approved 2026-09-29. Step 1 (foundation) is implemented.

## Summary

PV will replace the daemon's shell and Python test fixtures with `pv-fake`: one native Rust executable that emulates each Managed Resource and Gateway runtime pv supervises. Tests install it at each real executable path, e.g. `bin/caddy` or `bin/mysqld`, as a hard link. The fake reads its persona from a scenario file next to it, runs as a single process, and exits by itself when the test process that installed it dies.

Protocol layers come from established libraries wherever pv talks a real protocol. Each persona's answers are recorded from the real artifact first, and one contract suite runs against both the fake and, in the real-artifact lane, the real binary. That keeps the fakes from drifting.

This change is test-only. It does not change production daemon or supervisor behavior.

## Motivation

The chaos baseline (2026-09-29) measured the current fixtures:

- This laptop had 393 orphaned fixture processes plus 122 Python children left over from test runs between Sep 12 and Sep 26, using about 900 MB of memory and half a CPU core. All of them ran code from before the parent watchers (commit `91de9819`, merged in #356).
- Current code still leaks:
  - A normal full run of the daemon suite left 4 survivors.
  - Hard kills at 10 s, 25 s and 40 s left 6, 2 and 0.
  - The leaks come from fixtures outside #356's watcher convention: the `sleep "$1"` worker runtime (`crates/daemon/src/health.rs`) and the inline `FAKE_CADDY_SCRIPT` (`crates/daemon/tests/daemon_foundation.rs`). The `FAKE_CADDY_SCRIPT` leak is permanent.
  - The cooperative approach works where it was applied, but nothing enforces it.
- The full daemon suite is flaky under load. Each run has 1–3 failures in different tests, mostly "no PV-owned process could be verified", and every one passes in isolation. All current fakes are `#!` scripts, so they take the supervisor's script-identity path with its 250 ms stabilization; real binaries don't.
- Gateway fakes run as `sh → python` process pairs coordinated by `ps` polling, which is fragile under load. The Redis fake finds commands by substring rather than parsing RESP, so a TCP read split mid-command would break it.

## Goals

- Replace every long-running daemon test fixture with a `pv-fake` persona.
- Run each fake as one process that exits when its installing test process dies, including on SIGKILL.
- Take the direct executable-identity path that real binaries take, not the script fallback.
- Use a maintained library for every protocol and framing layer. Hand-written code only decides which answer to send.
- Derive each persona's behavior from recordings of the real artifact, and keep it honest with a contract suite that also runs against the real binary.
- Make fake failures legible: every fake writes an event log that tests can print on failure.
- Remove the `python3` test prerequisite once all fixtures are ported and the supervisor's script-identity tests are removed with the fallback they cover.

## Non-Goals

- Do not change production daemon, supervisor, or Gateway behavior. Removing the supervisor's script-identity fallback is a separate, later change.
- Do not emulate behavior pv never exercises. A persona covers the interactions pv performs plus the failure scenarios tests need.
- Do not replace the real-artifact lane. It becomes the second target of the contract suite.
- Do not add Linux support, the per-runtime monitor, or a simulated process backend.

## Architecture

### Personas and installation

`pv-fake` reads its persona from its scenario file, not from its executable name. Tests sometimes need one runtime's behavior under another's name, e.g. the health tests install a generic process as `bin/frankenphp`. Planned personas:

- `caddy`, `frankenphp`, `mysqld`, `postgres`, `initdb`, `redis-server`, `mailpit`, `rustfs`,
- the test-only variants tests use today, such as the positional fake Mailpit,
- `long_running`: a generic process that stays alive until SIGTERM or SIGINT. It replaces the ad-hoc `sleep` runtime and the inline fake SQL script, which is installed as `bin/pv-fake-sql`.

Tests install a fake with `pv_fake::install(executable, persona)`. It:

- sets up this test process's lifeline pipe (below) on first use,
- hard-links the `pv-fake` binary to `executable`, copying instead if the link would cross filesystems, and
- writes the scenario file next to it.

The link is a hard link, not a symlink, because PV's artifact validation (`RuntimeArtifactAdapter::validate_installation`, via `symlink_metadata`) rejects symlinked executables. That's a production policy the fakes must not work around.

A hard link is a regular file, and the process executable is the path the fake was started from. macOS reports the path passed to exec, not a resolved one (verified with a `KERN_PROCARGS2` probe), so the supervisor's `executable_matches` succeeds directly.

Control files that tests change while a fake is running keep their current names and meaning: `<config>.readiness-gate`, `<config>.readiness-fail`, `fake-admin-control.json` and the marker files. Ported tests only change their install helpers, not their bodies or snapshots.

### Binary location

Decided: the example target below.

`pv-fake` is a workspace crate containing a library (all logic) and a binary (used by its own contract tests through `CARGO_BIN_EXE_pv-fake`). The daemon lists it under `[dev-dependencies]`, so it never enters production builds, and adds a three-line example target, `crates/daemon/examples/pv-fake.rs`, that calls into the library.

Cargo builds a package's example targets whenever it builds the package's tests. It does so for `cargo nextest run -p daemon` with a test filter and for `cargo nextest run --workspace`. It does not for `cargo test --lib` (all three verified with a probe workspace). So the fake is rebuilt with the daemon's tests and can't silently go stale.

`pv_fake::binary()` locates it at `target/<profile>/examples/pv-fake`, relative to the running test executable, and returns a clear error when it's missing.

Cost: editing `pv-fake` rebuilds the daemon's test binaries, measured at about 5 seconds of incremental compile time.

Alternative considered: a standalone binary at `target/<profile>/pv-fake`. Cargo builds it for neither `-p daemon` nor `--workspace` (a package's binary is only built alongside its own integration tests), so local runs would silently use a stale or missing fake.

### Scenario file

Each installed fake has a scenario file next to it, `<executable>.pv-fake.json`, following today's `$0.server.py` sibling convention. The fake finds it from its absolute `argv[0]`. The install helper always writes one, so no environment variables are involved. A fake with no scenario file exits with an error naming the missing path.

It contains:

- the persona,
- the lifeline descriptor number,
- later, process-level settings: startup delay, readiness never or after N ms, crash after N ms with an exit code or signal, SIGTERM handling (exit, ignore, or delay), and spawning a descendant in the same or its own process group,
- later, persona settings where tests need them.

Settings are added only when a ported test needs them. Step 1 needs none beyond the persona and the lifeline.

### Event log

Each fake appends JSON lines to `<executable>.pv-fake.events.jsonl`. Events: started (persona, argv, whether the lifeline is armed), signal received, lifeline fired, and exit. Later steps add ready and descendant spawned. Every event carries a timestamp, pid, process group, and parent pid.

Tests read the log with `InstalledFake::events()`. Ported tests can add it to their failure context, so a failure shows whether pv or the fake misbehaved.

### Lifeline

On the first `pv_fake::install` call, each test process:

1. creates a pipe,
2. marks the write end close-on-exec right away. macOS has no `pipe2`, so this can't be atomic, and a process spawned in that instant would hold the pipe open,
3. keeps both ends in a process-lifetime static. The read end stays inheritable, and
4. records the read end's descriptor number in every scenario file it writes.

The number is whatever the pipe was given, never a fixed one, so nothing already open can be overwritten.

The fake `stat`s `/dev/fd/<n>` and checks for a FIFO. It then reads through that path on a blocking task, since opening `/dev/fd/<n>` duplicates the descriptor, so the fake never takes ownership of a raw descriptor number. The read returns end-of-file only when every write end is closed, which happens when the test process exits for any reason, SIGKILL included. It also returns at once if the test process died before the fake started. The fake then:

- signals its own process group with SIGKILL if it is the group leader, as supervised runtimes are. This mirrors the production validation anchor (`CONFIG_VALIDATION_GROUP_ANCHOR` in `crates/daemon/src/gateway.rs`).
- otherwise exits by itself, so a fake started directly by a contract test never signals the test's group.

Descendants are also `pv-fake` processes and inherit the descriptor, so a descendant in its own process group exits by itself too. If the descriptor isn't an open FIFO, for example when a fake is run by hand, no watcher starts.

This replaces every `ps`-polling parent watcher and the parent-capture test hooks.

### Process structure and signals

Each fake is one process with threads. Signal handling comes from the scenario file. Each persona's default SIGTERM behavior and exit status come from the recorded real artifact.

## Protocol Implementations

Rule: a library owns every protocol and framing layer. Persona code only decides what to answer.

| Persona | pv's interactions | Implementation |
|---|---|---|
| `caddy`, `frankenphp` | `validate`; `run --config … --adapter caddyfile`; HTTP and HTTPS using the configured PKI root certificate; admin Unix socket `GET /config/` and `POST /load` (`text/caddyfile`); `GET /__pv/health` over HTTP and TLS; the admin peer must be in the managed process group | `hyper`, `tokio-rustls` (workspace) |
| `postgres`, `initdb` | `sqlx` startup with `sslmode=disable`, `SELECT 1`, `SELECT 1 FROM pg_database WHERE datname = $1`, `CREATE DATABASE`, `SET`; `initdb` file layout | `pgwire` with `default-features = false` |
| `mysqld` | `--initialize-insecure`; TCP; `sqlx` `SELECT 1` and `CREATE DATABASE IF NOT EXISTS` | `opensrv-mysql`, pending the compatibility spike. It lets MySQL tests use the real `sqlx` client instead of `RecordingMysqlAdmin`. If the spike fails, keep `RecordingMysqlAdmin`; never hand-write the MySQL protocol |
| `redis-server` | `redis` crate multiplexed connection: `CLIENT SETINFO`, `PING` | `redis-protocol` for RESP framing, plus a command table |
| `rustfs` | `GET /health`; `aws-sdk-s3` `create_bucket` with path-style addressing and SigV4 using the configured credentials; reject mode | `s3s` + `s3s-fs`, the S3 layer the real RustFS is built on (`s3s` 0.17.0), with `hyper` for `/health` |
| `mailpit`, `pv-fake-mailpit` | SMTP greeting; HTTP readiness (`/`, `/ready`) | `tokio`, `hyper` |

These crates are dependencies of `pv-fake` only. Versions of shared crates come from the workspace.

## Contract Fidelity

1. **Record before implementing.** Before each persona is written, run the real artifact (installed from the artifact manifest, as `real_artifact_resource_matrix.rs` does) through exactly the interactions pv performs. Save what comes back as `insta` snapshots: status codes, relevant headers, body shapes (e.g. Caddy's `/load` error `{"error": …}`), CLI exit codes and stderr, startup parameters and greetings, and exit status after SIGTERM.
2. **One contract suite, two targets.** Persona contracts live in the daemon crate, e.g. `crates/daemon/tests/gateway_runtime_contracts.rs`. That way they drive each binary with PV's own renderers, `ProcessSupervisor`, admin client, and readiness checks, so the contract is exactly what PV depends on. Each runs against the fake by default. An ignored twin runs it against the real artifact, installed from the manifest, when `PV_E2E_REAL_ARTIFACTS=1` and `PV_E2E_ARTIFACT_MANIFEST_URL` are set. `.github/workflows/real-artifact-e2e.yml` runs the real twins, so an artifact update that changes behavior PV relies on fails there. Test configs for real Caddy add `skip_install_trust` so a fresh test CA never triggers a trust-store prompt.
3. **Failure scenarios are fake-only.** Crash on start, never ready, ignore SIGTERM, slow shutdown, and escaping descendants have no real-binary equivalent. Their contract tests run against the fake alone.
4. **Plumbing contracts.** Contract tests also cover:
   - process identity at the install path
   - the lifeline killing a group-leading fake, and only the fake itself otherwise, when the write end closes
   - a non-pipe descriptor leaving the lifeline unarmed
   - a missing scenario failing with an actionable error

   A daemon test starts a fake through the real supervisor. It checks that the lifeline arrives armed, that ownership verifies, and that no script executable identity is recorded.

The matching cases in `crates/daemon/tests/fixture_contracts.rs` move here as their fixtures are deleted.

## Recorded Behavior

### Caddy 2.11.4 (2026-09-30)

These were recorded with PV's Gateway config shape and a real test CA.

| Interaction | Real Caddy | `caddy` persona |
|---|---|---|
| `validate`, valid config | exit 0; JSON logs on stderr | exit 0 if the file exists |
| `validate`, missing file | exit 1; `Error: reading config from file: open <path>: no such file or directory` | same message and exit code |
| Root CA lifetime | refuses a root that expires before its 7-day intermediate | n/a |
| Admin socket | Unix socket, mode `0600` | same |
| `GET /config/` | `200 application/json`, full config | `200 application/json`, `{}` (PV checks only the status) |
| `/__pv/health` over HTTP | `200 text/plain` for `Host: pv-gateway.localhost`; `308` to HTTPS for other hosts | `200` for any host, body from the config's `respond` line |
| HTTPS | leaf per SNI (empty subject, critical SAN) signed by Caddy's own intermediate under PV's root | leaf per SNI signed directly by PV's root |
| `POST /load`, valid | `200`, JSON warnings array (PV's Caddyfile is not `caddy fmt` formatted) | `200`, empty body; PV accepts both |
| `POST /load`, invalid | `400 application/json` `{"error":"adapting config using caddyfile adapter: …"}` | not emulated; the stateful persona covers load failures |
| SIGTERM | graceful shutdown in milliseconds, exit 0 | same |

The persona issues leaves only when the configured certificate is a CA. For now it serves a non-CA certificate as-is, because daemon tests still seed a self-signed leaf as the "CA" and share it with Python Gateway fakes. Step 2b removes that fallback.

## Lints And Errors

`pv-fake` opts into workspace lints. It is an application boundary, so it uses `anyhow`. Raw process, filesystem, and executable-path primitives use narrow `#[expect(..., reason = "...")]` items, as elsewhere in the repository. It uses no `unwrap`, `expect`, or `panic!`.

## Implementation Sequence

Each step is one pull request. Each starts by recording the relevant real artifacts, and ends with the verification below.

1. **Foundation** (implemented).
   - Create the crate, the daemon example target and dev-dependency, `pv_fake::install`, the scenario file, the event log, the lifeline, and the plumbing contract tests.
   - Add the `long_running` persona, replacing the `health.rs` `sleep` runtime and the inline `fake_sql_script`.
   - Leave the supervisor tests that exercise script handling unchanged: `owned-python-runtime.py` and the `/bin/sh` descendant and argument scripts. They cover the supervisor's script-identity fallback and shell process groups, which still exist in production. They go when that fallback is removed.
2. **Gateway**, in two parts:
   - **2a** (implemented).
     - Record real Caddy (see Recorded Behavior).
     - Add the `caddy` and `frankenphp` personas and the dual-target Gateway and worker contracts.
     - Port the plain install sites in `gateway_reconciliation.rs` and `jobs.rs`, and the inline `FAKE_CADDY_SCRIPT` in `daemon_foundation.rs`.
   - **2b.**
     - Port `daemon_foundation.rs`'s barrier helpers, which patch fixture source text today, to scenario settings.
     - Port the stateful control-file, no-admin, admin-only and legacy variants.
     - Seed a real CA in every Gateway test and drop the TLS fallback below.
     - Remove `write_script_fake_frankenphp` in `gateway_reconciliation.rs`. It stays a script only because PV verifies a running script runtime by reading the file at its command path, and that test replaces a running stateful script fixture in place.
3. **Simple services.** Add `redis-server`, the three Mailpit variants, and `rustfs`.
4. **SQL.** Start with the `opensrv-mysql` + `sqlx` compatibility spike, then add `postgres`, `initdb`, the unready Postgres variant, and `mysqld`.
5. **Cleanup.**
   - Delete the remaining runtime-standing shell and Python fixtures, including the unreferenced `fake-frankenphp-hangs-on-port`.
   - Rewrite `CONTRIBUTING.md`'s Fixture Lifecycle section around `pv_fake::install` and the lifeline.
   - Remove its `python3` prerequisite once the script-identity tests are gone with their fallback.

## Verification

For every step:

1. Contract tests for new personas pass against the fake. Recorded snapshots are reviewed against the real artifact.
2. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and `cargo shear` pass.
3. The complete locked workspace nextest suite passes.
4. The chaos gauge reports zero survivors for ported fixtures after a normal run and after hard kills at several points in the run.
5. Several full daemon suite runs are compared against the baseline of 1–3 load-dependent failures per run.

## Risks And Mitigations

### Library compatibility

`opensrv-mysql` was last released in February 2024 and hasn't been checked against `sqlx` 0.9's connection sequence. Step 4 starts with a spike. The fallback is today's `RecordingMysqlAdmin`, not a hand-written protocol. `pgwire` and `s3s` are actively released. `s3s` is what the real RustFS uses.

### Stale or missing fake binary

`--lib`-only and `--test <name>` runs don't build examples. `pv_fake::binary()` fails with an actionable message when the binary is missing.

A stale binary is caught too. `build.rs` hashes the crate's sources into a build ID, `install` writes that ID into the scenario file, and a fake from a different build refuses to start with instructions to rebuild the examples.

### Descriptor inheritance

The lifeline assumes the supervisor's spawn path passes non-close-on-exec descriptors to children. A daemon test starts a fake through the real supervisor and verifies that the lifeline arrives armed. A targeted chaos run then hard-killed a test mid-start: the fake recorded `started` after its parent was already gone, and its lifeline fired within 63 µs.

### Symlink rejection

PV's artifact validation rejects symlinked executables. Fakes are hard links, or copies across filesystems, so they're regular files like real artifacts.

### Signaling the wrong process group

A fake only signals its own process group when it is the group leader. Otherwise it exits alone.

### Drift from real behavior

The dual-target contract suite runs in the real-artifact lane. Recorded snapshots are the reference.

### Compile time

Protocol crates are confined to `pv-fake`. The SQL crates arrive last. Editing the fake costs about 5 seconds of daemon test rebuild.

## Acceptance Criteria

The migration is complete when:

- every long-running daemon test fixture that stands in for a runtime is a `pv-fake` persona installed by `pv_fake::install`, and no `ps`-polling watcher remains,
- every persona's pv-facing behavior matches recorded real-artifact snapshots, and the contract suite runs against real binaries in the real-artifact lane,
- the chaos gauge reports zero survivors after normal runs and hard kills,
- `CONTRIBUTING.md` reflects the new fixture lifecycle, and
- formatting, Clippy, `cargo shear`, and the complete locked workspace nextest suite pass.

## Open Decisions

- MySQL: `opensrv-mysql`, if the step 4 spike succeeds.
